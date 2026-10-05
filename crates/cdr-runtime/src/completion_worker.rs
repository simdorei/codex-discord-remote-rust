use std::sync::Arc;
use std::time::Duration;

use cdr_app_server::goal::{ThreadGoalStatus, parse_thread_goal_update};
use cdr_app_server::outcomes::{
    TurnCompletion, TurnOutcomeError, TurnStatus, parse_turn_completion,
};
use cdr_app_server::{
    AppServerError, ResidentAppServer, ResidentNotificationEvent, extract_thread_id,
    extract_turn_id,
};
use cdr_store::StoreError;
use cdr_store::queue::{QueueJobState, list};
use thiserror::Error;
use tokio::sync::{Mutex, broadcast, watch};
use twilight_http::Client;

use crate::app_backend::AppServerTurnBackend;
use crate::commentary_stream::{CommentaryBlock, CommentaryBuffer};
use crate::queue_runner::{QueueCoordinator, QueueRunnerError};

mod context;
mod delivery;
mod delivery_identity;
mod delivery_order;
mod driver;
mod goal_progress;
mod history_request;
mod idle_release;
mod observation;
mod observation_gap;
mod receipt;
mod scheduler;
mod source_driver;
mod start_failure;
pub(crate) use start_failure::deliver_start_failures;
mod recovery;
mod terminal_fence;
mod typing;

#[cfg(test)]
mod early_journal_tests;
#[cfg(test)]
mod final_answer_fallback_tests;
#[cfg(test)]
mod first_reply_order_tests;
#[cfg(test)]
mod goal_handoff_boundary_tests;
#[cfg(test)]
mod goal_handoff_tests;
#[cfg(test)]
mod goal_mirror_boundary_tests;
#[cfg(test)]
mod idle_protocol_tests;
#[cfg(test)]
mod idle_release_tests;
#[cfg(test)]
mod new_attachment_tests;
#[cfg(test)]
mod new_first_reply_recovery_tests;
#[cfg(test)]
mod new_first_reply_tests;
#[cfg(test)]
mod reserve_inheritance_tests;

pub use delivery::completion_message;
use delivery::i64_channel;
use delivery_identity::CompletionDeliveryIdentity;
pub(crate) use delivery_identity::IdempotentChunk;
pub(crate) use receipt::send_chunk as send_recorded_message_chunk;
pub(crate) use receipt::send_chunk_with_components as send_recorded_message_with_components;

#[derive(Debug, Error)]
pub enum CompletionWorkerError {
    #[error(transparent)]
    AppServer(#[from] AppServerError),
    #[error(transparent)]
    Outcome(#[from] TurnOutcomeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Queue(#[from] QueueRunnerError),
    #[error("invalid thread goal response: {0}")]
    Goal(String),
    #[error("Discord final delivery failed: {0}")]
    Delivery(String),
    #[error("output held without HTTP attempt: {0}")]
    Held(String),
    #[error("Discord channel identifier does not fit the unsigned contract")]
    ChannelId,
}

pub async fn run_completion_worker(
    receiver: broadcast::Receiver<ResidentNotificationEvent>,
    server: Arc<ResidentAppServer>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    http: Arc<Client>,
    commentary_enabled: bool,
    history_read_timeout: Duration,
    shutdown: watch::Receiver<bool>,
) {
    let worker = Arc::new(CompletionWorker {
        server,
        queue,
        http,
        commentary_enabled,
        history_read_timeout,
        commentary: Mutex::new(CommentaryBuffer::default()),
        terminal_fence: terminal_fence::TerminalFence::default(),
    });
    driver::run(worker, receiver, shutdown).await;
}

#[derive(Clone, Copy)]
enum Processing<'a> {
    Inline,
    Staged(&'a StateAdmission<'a>),
}

struct StateAdmission<'a> {
    lease: crate::queue_runner::TargetLease<'a, AppServerTurnBackend>,
    owner: AdmissionOwner,
}

enum AdmissionOwner {
    NotTerminal,
    Missing,
    Exact(Box<cdr_store::queue::StoredQueueJob>),
}

impl<'a> Processing<'a> {
    fn is_inline(self) -> bool {
        matches!(self, Self::Inline)
    }
    fn admission(self) -> Option<&'a StateAdmission<'a>> {
        match self {
            Self::Inline => None,
            Self::Staged(admission) => Some(admission),
        }
    }
    fn validate_owner(
        self,
        owner: &cdr_store::queue::StoredQueueJob,
    ) -> Result<(), CompletionWorkerError> {
        if let Some(admission) = self.admission() {
            let matches = match &admission.owner {
                AdmissionOwner::NotTerminal => true,
                AdmissionOwner::Missing => false,
                AdmissionOwner::Exact(expected) => expected.as_ref() == owner,
            };
            if !matches {
                return Err(CompletionWorkerError::Held(
                    "completion admission owner changed".into(),
                ));
            }
        }
        Ok(())
    }
}

struct CompletionWorker {
    server: Arc<ResidentAppServer>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    http: Arc<Client>,
    commentary_enabled: bool,
    history_read_timeout: Duration,
    commentary: Mutex<CommentaryBuffer>,
    terminal_fence: terminal_fence::TerminalFence,
}

impl CompletionWorker {
    fn release_generation(
        &self,
        completion: &TurnCompletion,
        generation: i64,
    ) -> Result<Option<i64>, CompletionWorkerError> {
        let confirmed = cdr_store::observed_completion::has_resident_evidence(
            self.queue.db_path(),
            &completion.thread_id,
            &completion.turn_id,
            generation,
            self.server.instance_id(),
        )?;
        Ok(confirmed.then_some(generation))
    }
    async fn deliver_questions(&self) -> Result<(), CompletionWorkerError> {
        crate::async_question_ui::deliver_pending(
            self.queue.db_path(),
            self.server.instance_id(),
            self.server.generation(),
            &self.http,
        )
        .await
    }

    #[cfg(test)]
    async fn handle(&self, event: ResidentNotificationEvent) -> Result<(), CompletionWorkerError> {
        self.handle_mode(event, Processing::Inline).await
    }

    async fn handle_mode(
        &self,
        event: ResidentNotificationEvent,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        // The observer may precede Goal attachment; FIFO processing can now
        // record the exact owned event. INSERT OR IGNORE preserves first evidence.
        self.observe_terminal(&event)?;
        let ResidentNotificationEvent::Notification {
            generation,
            notification,
        } = event
        else {
            return if mode.is_inline() {
                self.recover().await
            } else {
                Ok(())
            };
        };
        if notification.method == "item/completed"
            && notification
                .params
                .get("item")
                .is_some_and(cdr_app_server::async_questions::is_async_message)
        {
            return if mode.is_inline() {
                self.deliver_questions().await
            } else {
                self.queue.notify_delivery_ready();
                Ok(())
            };
        }
        if self.commentary_enabled {
            let block = self
                .commentary
                .lock()
                .await
                .observe(&notification.method, &notification.params);
            if let Some(block) = block {
                self.stage_commentary(&block, mode).await?;
            }
        }
        match notification.method.as_str() {
            "turn/started" => {
                if let (Some(thread), Some(turn)) = (
                    extract_thread_id(&notification.params),
                    extract_turn_id(&notification.params),
                ) {
                    cdr_store::async_question::supersede(
                        self.queue.db_path(),
                        self.server.instance_id(),
                        i64::try_from(generation).map_err(|_| QueueRunnerError::IntegerRange)?,
                        &thread,
                        &turn,
                    )?;
                    let _ = if let Some(admission) = mode.admission() {
                        admission
                            .lease
                            .goal_turn_started_observed(&turn, generation, None)?
                    } else {
                        self.queue
                            .goal_turn_started_observed(&thread, &turn, generation, None)
                            .await?
                    };
                    // The observer may already have journalled this successor's
                    // questions while goal-progress delivery delayed the handoff.
                    if mode.is_inline() {
                        self.deliver_questions().await?;
                    } else {
                        cdr_store::async_question::reconcile_observations(
                            self.queue.db_path(),
                            self.server.instance_id(),
                            i64::try_from(self.server.generation())
                                .map_err(|_| QueueRunnerError::IntegerRange)?,
                        )?;
                        self.queue.notify_delivery_ready();
                    }
                }
            }
            "turn/completed" => {
                let completion = parse_turn_completion(&notification.params, false)?;
                let evidence_generation =
                    i64::try_from(generation).map_err(|_| QueueRunnerError::IntegerRange)?;
                self.finish_with_owner_mode(
                    generation,
                    evidence_generation,
                    &completion,
                    None,
                    mode,
                )
                .await?;
            }
            "thread/goal/updated" => {
                let update = parse_thread_goal_update(&notification.params)
                    .map_err(|error| CompletionWorkerError::Goal(error.to_string()))?;
                if update.status != ThreadGoalStatus::Active {
                    self.finish_waiting_goal_mode(generation, &update.thread_id, mode)
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    #[cfg(test)]
    async fn finish(
        &self,
        server_generation: u64,
        evidence_generation: i64,
        completion: &TurnCompletion,
    ) -> Result<(), CompletionWorkerError> {
        self.finish_with_owner(server_generation, evidence_generation, completion, None)
            .await
    }

    #[cfg(test)]
    async fn finish_with_owner(
        &self,
        server_generation: u64,
        evidence_generation: i64,
        completion: &TurnCompletion,
        expected_owner: Option<&cdr_store::queue::StoredQueueJob>,
    ) -> Result<(), CompletionWorkerError> {
        self.finish_with_owner_mode(
            server_generation,
            evidence_generation,
            completion,
            expected_owner,
            Processing::Inline,
        )
        .await
    }

    async fn finish_with_owner_mode(
        &self,
        server_generation: u64,
        evidence_generation: i64,
        completion: &TurnCompletion,
        expected_owner: Option<&cdr_store::queue::StoredQueueJob>,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        let Some(owner) = list(self.queue.db_path())?.into_iter().find(|job| {
            job.state == QueueJobState::Running
                && job.target_thread_id == completion.thread_id
                && job.turn_id.as_deref() == Some(completion.turn_id.as_str())
        }) else {
            if expected_owner.is_some() || !mode.is_inline() {
                return Err(CompletionWorkerError::Held(
                    "captured completion owner no longer exists".into(),
                ));
            }
            cdr_store::observed_completion::finish(
                self.queue.db_path(),
                &completion.thread_id,
                &completion.turn_id,
            )?;
            return Ok(());
        };
        mode.validate_owner(&owner)?;
        if expected_owner.is_some_and(|expected| expected != &owner) {
            return Err(CompletionWorkerError::Held(
                "captured completion ownership changed".into(),
            ));
        }
        self.commentary
            .lock()
            .await
            .discard_turn(&completion.thread_id, &completion.turn_id);
        let goal = if completion.status == TurnStatus::Completed {
            self.goal_status(server_generation, &completion.thread_id)
                .await?
        } else {
            None
        };
        // This gate is shared by periodic recovery, Goal updates, and duplicate
        // terminal notifications; none may turn prior progress into a false Final.
        if owner.goal_waiting && goal != Some(ThreadGoalStatus::Active) {
            let confirmed = self
                .waiting_goal_completion(server_generation, &owner)
                .await?;
            if confirmed.status != completion.status {
                return Err(CompletionWorkerError::Held(
                    "waiting terminal status changed".into(),
                ));
            }
        }
        let reply = if completion.status == TurnStatus::Completed {
            self.exact_text(
                server_generation,
                evidence_generation,
                completion,
                &owner,
                (goal.is_some() || owner.goal_waiting) && goal != Some(ThreadGoalStatus::Active),
            )
            .await?
        } else {
            context::ExactReply {
                text: Some(String::new()),
                needs_goal_handoff: false,
            }
        };
        let continues = goal == Some(ThreadGoalStatus::Active) || reply.needs_goal_handoff;
        let Some(exact) = reply.text else {
            return self
                .finish_without_exact_reply(
                    completion,
                    if continues {
                        Some(ThreadGoalStatus::Active)
                    } else {
                        goal
                    },
                    &owner,
                    evidence_generation,
                    mode,
                )
                .await;
        };
        // A known successor makes this exact turn progress even when Goal/get
        // already reports complete. Retain J until FIFO validates its next start.
        if continues {
            let text = if exact.is_empty() {
                String::new()
            } else {
                format!("[Goal progress]\n{exact}")
            };
            if let Some(pending) = self.stage_progress_mode(&owner, &text, mode).await? {
                self.queue.notify_delivery_ready();
                if mode.is_inline() {
                    self.deliver_goal_progress(&pending).await?;
                }
            }
            return Ok(());
        }
        self.finish_owned_terminal(&owner, completion, &exact, goal, evidence_generation, mode)
            .await
    }

    async fn finish_owned_terminal(
        &self,
        owner: &cdr_store::queue::StoredQueueJob,
        completion: &TurnCompletion,
        exact: &str,
        goal: Option<ThreadGoalStatus>,
        evidence_generation: i64,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        let text = completion_message(completion, exact, goal);
        self.stage_terminal(owner, completion, &text, evidence_generation, mode)
            .await?;
        if mode.is_inline()
            && self
                .running_channel(&completion.thread_id, &completion.turn_id)?
                .is_none()
        {
            cdr_store::observed_completion::finish(
                self.queue.db_path(),
                &completion.thread_id,
                &completion.turn_id,
            )?;
        }
        Ok(())
    }

    async fn finish_without_exact_reply(
        &self,
        completion: &TurnCompletion,
        goal: Option<ThreadGoalStatus>,
        owner: &cdr_store::queue::StoredQueueJob,
        evidence_generation: i64,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        let text = "ERROR: Codex turn completed, but its exact final reply could not be recovered: \
thread/read did not contain the requested turn and no matching final-answer event was stored.";
        if goal == Some(ThreadGoalStatus::Active) {
            if let Some(pending) = self.stage_progress_mode(owner, text, mode).await? {
                self.queue.notify_delivery_ready();
                if mode.is_inline() {
                    self.deliver_goal_progress(&pending).await?;
                }
            }
            return Ok(());
        }
        self.stage_terminal(owner, completion, text, evidence_generation, mode)
            .await?;
        Ok(())
    }

    async fn stage_progress_mode(
        &self,
        owner: &cdr_store::queue::StoredQueueJob,
        text: &str,
        mode: Processing<'_>,
    ) -> Result<Option<cdr_store::goal_progress::PendingProgress>, CompletionWorkerError> {
        Ok(if let Some(admission) = mode.admission() {
            admission.lease.stage_owned_goal_progress(owner, text)?
        } else {
            self.queue.stage_owned_goal_progress(owner, text).await?
        })
    }

    async fn stage_terminal(
        &self,
        owner: &cdr_store::queue::StoredQueueJob,
        completion: &TurnCompletion,
        text: &str,
        generation: i64,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        let observed = self.release_generation(completion, generation)?;
        let pending = if mode.is_inline() {
            self.queue
                .stage_owned_turn_completion_observed(owner, text, completion.usage_limit, observed)
                .await?
        } else {
            mode.admission()
                .expect("staged admission")
                .lease
                .save_owned_turn_completion_observed(owner, text, observed)
                .await?
        };
        if let Some(delivery) = pending
            && mode.is_inline()
        {
            self.deliver_one(&delivery).await?;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn send_commentary(&self, block: &CommentaryBlock) -> Result<(), CompletionWorkerError> {
        self.stage_commentary(block, Processing::Inline).await
    }

    async fn stage_commentary(
        &self,
        block: &CommentaryBlock,
        mode: Processing<'_>,
    ) -> Result<(), CompletionWorkerError> {
        let Some(pending) = cdr_store::commentary_outbox::stage(
            self.queue.db_path(),
            &block.thread_id,
            &block.turn_id,
            &block.text,
        )?
        else {
            return Ok(());
        };
        self.queue.notify_delivery_ready();
        if mode.is_inline() {
            self.deliver_commentary(&pending).await
        } else {
            Ok(())
        }
    }

    fn running_channel(&self, thread_id: &str, turn_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(list(self.queue.db_path())?
            .into_iter()
            .find(|job| {
                job.state == QueueJobState::Running
                    && job.target_thread_id == thread_id
                    && job.turn_id.as_deref() == Some(turn_id)
            })
            .map(|job| job.channel_id))
    }
}

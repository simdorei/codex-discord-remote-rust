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
mod receipt;
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
    async fn deliver_questions(&self) -> Result<(), CompletionWorkerError> {
        crate::async_question_ui::deliver_pending(
            self.queue.db_path(),
            self.server.instance_id(),
            self.server.generation(),
            &self.http,
        )
        .await
    }

    async fn handle(&self, event: ResidentNotificationEvent) -> Result<(), CompletionWorkerError> {
        let ResidentNotificationEvent::Notification {
            generation,
            notification,
        } = event
        else {
            return self.recover().await;
        };
        if notification.method == "item/completed"
            && notification
                .params
                .get("item")
                .is_some_and(cdr_app_server::async_questions::is_async_message)
        {
            return self.deliver_questions().await;
        }
        if self.commentary_enabled {
            let block = self
                .commentary
                .lock()
                .await
                .observe(&notification.method, &notification.params);
            if let Some(block) = block {
                self.send_commentary(&block).await?;
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
                    let _ = self.queue.goal_turn_started(&thread, &turn).await?;
                    // The observer may already have journalled this successor's
                    // questions while goal-progress delivery delayed the handoff.
                    self.deliver_questions().await?;
                }
            }
            "turn/completed" => {
                let completion = parse_turn_completion(&notification.params, false)?;
                let evidence_generation =
                    i64::try_from(generation).map_err(|_| QueueRunnerError::IntegerRange)?;
                self.finish(generation, evidence_generation, &completion)
                    .await?;
            }
            "thread/goal/updated" => {
                let update = parse_thread_goal_update(&notification.params)
                    .map_err(|error| CompletionWorkerError::Goal(error.to_string()))?;
                if update.status != ThreadGoalStatus::Active {
                    self.finish_waiting_goal(generation, &update.thread_id)
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn finish(
        &self,
        server_generation: u64,
        evidence_generation: i64,
        completion: &TurnCompletion,
    ) -> Result<(), CompletionWorkerError> {
        if self
            .running_channel(&completion.thread_id, &completion.turn_id)?
            .is_none()
        {
            cdr_store::observed_completion::finish(
                self.queue.db_path(),
                &completion.thread_id,
                &completion.turn_id,
            )?;
            return Ok(());
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
        let exact = if completion.status == TurnStatus::Completed {
            match self
                .exact_text(server_generation, evidence_generation, completion)
                .await
            {
                Ok(text) => text,
                Err(CompletionWorkerError::Outcome(TurnOutcomeError::TurnNotFound)) => {
                    return self
                        .finish_without_exact_reply(completion, goal, evidence_generation)
                        .await;
                }
                Err(error) => return Err(error),
            }
        } else {
            String::new()
        };
        if goal == Some(ThreadGoalStatus::Active) {
            let text = if exact.is_empty() {
                String::new()
            } else {
                format!("[Goal progress]\n{exact}")
            };
            if let Some(pending) = self
                .queue
                .stage_goal_progress(&completion.thread_id, &completion.turn_id, &text)
                .await?
            {
                self.deliver_goal_progress(&pending).await?;
            }
            return Ok(());
        }
        let text = completion_message(completion, &exact, goal);
        if let Some(delivery) = self
            .queue
            .stage_turn_completion_on_generation(
                &completion.thread_id,
                &completion.turn_id,
                &text,
                evidence_generation,
            )
            .await?
        {
            self.deliver_one(&delivery).await?;
        }
        if self
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
        evidence_generation: i64,
    ) -> Result<(), CompletionWorkerError> {
        let text = "ERROR: Codex turn completed, but its exact final reply could not be recovered: \
thread/read did not contain the requested turn and no matching final-answer event was stored.";
        if goal == Some(ThreadGoalStatus::Active) {
            if let Some(pending) = self
                .queue
                .stage_goal_progress(&completion.thread_id, &completion.turn_id, text)
                .await?
            {
                self.deliver_goal_progress(&pending).await?;
            }
            return Ok(());
        }
        if let Some(delivery) = self
            .queue
            .stage_turn_completion_on_generation(
                &completion.thread_id,
                &completion.turn_id,
                text,
                evidence_generation,
            )
            .await?
        {
            self.deliver_one(&delivery).await?;
        }
        Ok(())
    }

    async fn send_commentary(&self, block: &CommentaryBlock) -> Result<(), CompletionWorkerError> {
        let Some(pending) = cdr_store::commentary_outbox::stage(
            self.queue.db_path(),
            &block.thread_id,
            &block.turn_id,
            &block.text,
        )?
        else {
            return Ok(());
        };
        self.deliver_commentary(&pending).await
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

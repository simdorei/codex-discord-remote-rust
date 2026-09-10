use std::collections::{BTreeMap, BTreeSet};

use cdr_store::queue::{
    AppServerForkHandoff, NewAppServerForkHandoff, QueueJobState, begin_app_server_fork_handoff,
    completed_app_server_fork_target_for_source, finalize_app_server_fork_handoff,
    is_app_server_managed_target, list, list_filtered,
    record_and_cancel_app_server_fork_handoff_after_definite_failure,
    record_app_server_fork_failure, record_app_server_fork_finalize_failure,
    stage_app_server_fork_target, unresolved_app_server_fork_handoff_for_source,
};
use uuid::Uuid;

use super::{QueueCoordinator, QueueRunnerError, TurnBackend, generation_i64};

const PROACTIVE_REASON: &str = "app-server-only ownership fork";
const WRITER_CONFLICT_REASON: &str = "app-server active-writer ownership fork";
const INTERRUPTED_FORK_ERROR: &str =
    "app-server fork handoff was interrupted before a response was durably recorded";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerTarget {
    pub thread_id: String,
    pub forked_from: Option<String>,
    pub quarantined_job_id: Option<String>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    pub async fn ensure_app_server_only_target(
        &self,
        source_thread_id: &str,
    ) -> Result<AppServerTarget, QueueRunnerError> {
        self.handoff_target(source_thread_id, false, PROACTIVE_REASON)
            .await
    }

    pub async fn force_app_server_only_target(
        &self,
        source_thread_id: &str,
    ) -> Result<AppServerTarget, QueueRunnerError> {
        self.handoff_target(source_thread_id, true, WRITER_CONFLICT_REASON)
            .await
    }

    async fn handoff_target(
        &self,
        source_thread_id: &str,
        force: bool,
        reason: &str,
    ) -> Result<AppServerTarget, QueueRunnerError> {
        self.ensure_target_not_held(source_thread_id)?;
        if !self.backend.requires_app_server_fork() {
            return Ok(unchanged(source_thread_id));
        }
        let source_lock = self.target_lock(source_thread_id)?;
        let _source_guard = source_lock.lock().await;
        if let Some(target) = self.completed_target_chain(source_thread_id)? {
            return Ok(AppServerTarget {
                thread_id: target,
                forked_from: Some(source_thread_id.to_owned()),
                quarantined_job_id: None,
            });
        }
        if let Some(existing) =
            unresolved_app_server_fork_handoff_for_source(&self.db_path, source_thread_id)?
        {
            if let Some(target) = existing.observed_target_thread_id.clone() {
                return self
                    .finalize_staged_target(source_thread_id, &existing.handoff_id, &target)
                    .await;
            }
            refresh_unresolved_fork_notice(&self.db_path, &existing)?;
            return Err(QueueRunnerError::UnresolvedForkHandoff {
                source_thread_id: source_thread_id.to_owned(),
                handoff_id: existing.handoff_id,
                last_fork_error: visible_fork_error(&existing.last_fork_error),
            });
        }
        if !force && is_app_server_managed_target(&self.db_path, source_thread_id)? {
            return Ok(unchanged(source_thread_id));
        }

        let jobs = list_filtered(&self.db_path, Some(source_thread_id), None)?;
        let ambiguous = jobs.iter().find(|job| job.state == QueueJobState::Starting);
        let generation = ambiguous.map_or(generation_i64(self.backend.generation())?, |job| {
            job.app_server_generation
        });
        let handoff_id = Uuid::new_v4().to_string();
        let begun = begin_app_server_fork_handoff(
            &self.db_path,
            NewAppServerForkHandoff {
                handoff_id: &handoff_id,
                ambiguous_job_id: ambiguous.map(|job| job.job_id.as_str()),
                source_thread_id,
                expected_generation: generation,
                quarantine_reason: reason,
            },
        )?;
        if !begun.created {
            if let Some(target) = begun.handoff.target_thread_id.clone() {
                return Ok(AppServerTarget {
                    thread_id: target,
                    forked_from: Some(source_thread_id.to_owned()),
                    quarantined_job_id: begun.handoff.ambiguous_job_id,
                });
            }
            if let Some(target) = begun.handoff.observed_target_thread_id.clone() {
                return self
                    .finalize_staged_target(source_thread_id, &begun.handoff.handoff_id, &target)
                    .await;
            }
            refresh_unresolved_fork_notice(&self.db_path, &begun.handoff)?;
            return Err(QueueRunnerError::UnresolvedForkHandoff {
                source_thread_id: source_thread_id.to_owned(),
                handoff_id: begun.handoff.handoff_id,
                last_fork_error: visible_fork_error(&begun.handoff.last_fork_error),
            });
        }

        let target_thread_id = self
            .request_and_stage_fork(source_thread_id, &begun.handoff)
            .await?;
        self.finalize_staged_target(source_thread_id, &handoff_id, &target_thread_id)
            .await
    }

    async fn request_and_stage_fork(
        &self,
        source_thread_id: &str,
        handoff: &AppServerForkHandoff,
    ) -> Result<String, QueueRunnerError> {
        let target_thread_id = match self.backend.fork_thread(source_thread_id).await {
            Ok(target) => target,
            Err(failure) => {
                let recorded = if failure.ambiguous {
                    record_app_server_fork_failure(
                        &self.db_path,
                        &handoff.handoff_id,
                        &failure.message,
                        true,
                    )
                    .map(|_| ())
                } else {
                    record_and_cancel_app_server_fork_handoff_after_definite_failure(
                        &self.db_path,
                        handoff,
                        &failure.message,
                    )
                    .map(|_| ())
                };
                if let Err(recording) = recorded {
                    return Err(QueueRunnerError::ForkFailureRecording {
                        source_thread_id: source_thread_id.to_owned(),
                        handoff_id: handoff.handoff_id.clone(),
                        failure,
                        recording: Box::new(recording),
                    });
                }
                return Err(QueueRunnerError::ForkBackend {
                    source_thread_id: source_thread_id.to_owned(),
                    handoff_id: handoff.handoff_id.clone(),
                    failure,
                });
            }
        };
        if let Err(staging) =
            stage_app_server_fork_target(&self.db_path, &handoff.handoff_id, &target_thread_id)
        {
            return Err(QueueRunnerError::ForkTargetStage {
                source_thread_id: source_thread_id.to_owned(),
                handoff_id: handoff.handoff_id.clone(),
                target_thread_id,
                staging: Box::new(staging),
            });
        }
        Ok(target_thread_id)
    }

    async fn finalize_staged_target(
        &self,
        source_thread_id: &str,
        handoff_id: &str,
        target_thread_id: &str,
    ) -> Result<AppServerTarget, QueueRunnerError> {
        let target_lock = self.target_lock(target_thread_id)?;
        let _target_guard = target_lock.lock().await;
        let completed = match finalize_app_server_fork_handoff(
            &self.db_path,
            handoff_id,
            generation_i64(self.backend.generation())?,
        ) {
            Ok(completed) => completed,
            Err(failure) => {
                let failure_message = failure.to_string();
                if let Err(recording) = record_app_server_fork_finalize_failure(
                    &self.db_path,
                    handoff_id,
                    &failure_message,
                ) {
                    return Err(QueueRunnerError::ForkFinalizeRecording {
                        source_thread_id: source_thread_id.to_owned(),
                        handoff_id: handoff_id.to_owned(),
                        target_thread_id: target_thread_id.to_owned(),
                        failure: Box::new(failure),
                        recording: Box::new(recording),
                    });
                }
                return Err(QueueRunnerError::ForkFinalize {
                    source_thread_id: source_thread_id.to_owned(),
                    handoff_id: handoff_id.to_owned(),
                    target_thread_id: target_thread_id.to_owned(),
                    failure: Box::new(failure),
                });
            }
        };
        let quarantined_job_id = completed.quarantined_job.map(|job| job.job_id);
        eprintln!(
            "rust_queue_app_server_fork source={source_thread_id} target={target_thread_id} quarantined_job={}",
            quarantined_job_id.as_deref().unwrap_or("none")
        );
        Ok(AppServerTarget {
            thread_id: target_thread_id.to_owned(),
            forked_from: Some(source_thread_id.to_owned()),
            quarantined_job_id,
        })
    }

    fn completed_target_chain(
        &self,
        source_thread_id: &str,
    ) -> Result<Option<String>, QueueRunnerError> {
        let mut seen = BTreeSet::from([source_thread_id.to_owned()]);
        let mut current = source_thread_id.to_owned();
        while let Some(next) = completed_app_server_fork_target_for_source(&self.db_path, &current)?
        {
            self.ensure_target_not_held(&next)?;
            if !seen.insert(next.clone()) {
                return Err(QueueRunnerError::ForkTargetCycle {
                    source_thread_id: source_thread_id.to_owned(),
                });
            }
            current = next;
        }
        Ok((current != source_thread_id).then_some(current))
    }

    pub(super) async fn prepare_unmanaged_targets(&self) -> Result<(), QueueRunnerError> {
        if !self.backend.requires_app_server_fork() {
            return Ok(());
        }
        let mut targets = BTreeMap::<String, Vec<QueueJobState>>::new();
        for job in list(&self.db_path)? {
            if job.state != QueueJobState::Quarantined {
                targets
                    .entry(job.target_thread_id)
                    .or_default()
                    .push(job.state);
            }
        }
        for (target, states) in targets {
            if cdr_store::dead_generation::target_is_held(&self.db_path, &target)? {
                continue;
            }
            let has_unresolved =
                unresolved_app_server_fork_handoff_for_source(&self.db_path, &target)?.is_some();
            if !has_unresolved
                && states
                    .iter()
                    .any(|state| matches!(state, QueueJobState::Starting | QueueJobState::Running))
            {
                continue;
            }
            if let Err(error) = self.ensure_app_server_only_target(&target).await {
                let durably_fenced =
                    unresolved_app_server_fork_handoff_for_source(&self.db_path, &target)?
                        .is_some();
                if is_nonfatal_recovery_blocker(&error) && durably_fenced {
                    eprintln!("rust_queue_app_server_fork_blocked target={target} error={error}");
                    continue;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) async fn fork_writer_conflict_if_safe(
        &self,
        target: &str,
    ) -> Result<bool, QueueRunnerError> {
        let jobs = list_filtered(&self.db_path, Some(target), None)?;
        if jobs.is_empty()
            || jobs
                .iter()
                .any(|job| matches!(job.state, QueueJobState::Starting | QueueJobState::Running))
        {
            return Ok(false);
        }
        let _ = self.force_app_server_only_target(target).await?;
        Ok(true)
    }

    pub(super) fn ensure_target_not_held(&self, target: &str) -> Result<(), QueueRunnerError> {
        if cdr_store::dead_generation::target_is_held(&self.db_path, target)? {
            return Err(cdr_store::StoreError::DeadGenerationTargetHeld(target.into()).into());
        }
        Ok(())
    }
}

fn unchanged(thread_id: &str) -> AppServerTarget {
    AppServerTarget {
        thread_id: thread_id.to_owned(),
        forked_from: None,
        quarantined_job_id: None,
    }
}

pub(super) fn is_nonfatal_recovery_blocker(error: &QueueRunnerError) -> bool {
    match error {
        QueueRunnerError::ForkHandoff(cdr_store::queue::AppServerForkHandoffError::Store(_)) => {
            false
        }
        QueueRunnerError::ForkBackend { .. }
        | QueueRunnerError::ForkCancellation { .. }
        | QueueRunnerError::ForkFinalize { .. }
        | QueueRunnerError::UnresolvedForkHandoff { .. }
        | QueueRunnerError::ForkHandoff(_) => true,
        _ => false,
    }
}

fn refresh_unresolved_fork_notice(
    db_path: &std::path::Path,
    handoff: &cdr_store::queue::AppServerForkHandoff,
) -> Result<(), QueueRunnerError> {
    let error = visible_fork_error(&handoff.last_fork_error);
    record_app_server_fork_failure(
        db_path,
        &handoff.handoff_id,
        &error,
        handoff.fork_failure_ambiguous || handoff.last_fork_error.is_empty(),
    )?;
    Ok(())
}

fn visible_fork_error(error: &str) -> String {
    if error.is_empty() {
        INTERRUPTED_FORK_ERROR.to_owned()
    } else {
        error.to_owned()
    }
}

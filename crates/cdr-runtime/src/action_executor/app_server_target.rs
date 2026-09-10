use std::collections::BTreeSet;

use cdr_store::mapping::mirrored_thread_id;
use cdr_store::queue::completed_app_server_fork_target_for_source;

use super::{ActionError, ActionExecutor};
use crate::queue_runner::{BackendFailureKind, Submission, TurnBackend};

pub(super) struct ActionTarget {
    pub thread_id: String,
    pub source_label: String,
    pub mirror_mapping: bool,
}

impl<B: TurnBackend> ActionExecutor<B> {
    pub(super) async fn prepare_action_target(
        &self,
        thread_id: &str,
        source: &str,
    ) -> Result<ActionTarget, ActionError> {
        let canonical = self.canonicalize_completed_target(thread_id)?;
        let was_canonicalized = canonical != thread_id;
        let target = self.queue.ensure_app_server_only_target(&canonical).await?;
        self.apply_fork_state(&target)?;
        Ok(ActionTarget {
            source_label: fork_source_label(
                source,
                was_canonicalized || target.forked_from.is_some(),
            ),
            thread_id: target.thread_id,
            mirror_mapping: source == "mirror",
        })
    }

    pub(super) fn canonicalize_completed_target(
        &self,
        thread_id: &str,
    ) -> Result<String, ActionError> {
        if !self.queue.backend.requires_app_server_fork() {
            return Ok(thread_id.to_owned());
        }
        let mut current = thread_id.to_owned();
        let mut visited = BTreeSet::new();
        while visited.insert(current.clone()) {
            let Some(target) =
                completed_app_server_fork_target_for_source(&self.mirror_db, &current)?
            else {
                return Ok(current);
            };
            self.bridge_state.apply_thread_fork(&current, &target)?;
            current = target;
        }
        Err(ActionError::Invalid(format!(
            "app-server fork handoff cycle detected for {thread_id}"
        )))
    }

    pub(super) async fn recover_active_writer_submission(
        &self,
        target: ActionTarget,
        submission: Submission,
    ) -> Result<(ActionTarget, Submission), ActionError> {
        if !self.queue.backend.requires_app_server_fork() {
            return Ok((target, submission));
        }
        if submission.warning.as_ref().map(|warning| warning.kind)
            != Some(BackendFailureKind::ActiveWriter)
        {
            return Ok((target, submission));
        }
        let moved = self
            .queue
            .force_app_server_only_target(&target.thread_id)
            .await?;
        self.apply_fork_state(&moved)?;
        let _ = self.queue.recover_target(&moved.thread_id).await?;
        let refreshed = self
            .queue
            .replay_submission_for_job(&submission.job_id)?
            .ok_or_else(|| {
                ActionError::Invalid(format!(
                    "queue job disappeared after app-server fork: {}",
                    submission.job_id
                ))
            })?;
        Ok((
            ActionTarget {
                thread_id: moved.thread_id,
                source_label: fork_source_label(&target.source_label, true),
                mirror_mapping: target.mirror_mapping,
            },
            refreshed,
        ))
    }

    pub(super) fn current_mirror_target(
        &self,
        channel_id: i64,
        fallback: &str,
    ) -> Result<(String, &'static str), ActionError> {
        Ok(
            mirrored_thread_id(&self.mirror_db, Some(channel_id))?.map_or_else(
                || (fallback.to_owned(), "selected"),
                |target| (target, "mirror"),
            ),
        )
    }

    fn apply_fork_state(
        &self,
        target: &crate::queue_runner::AppServerTarget,
    ) -> Result<(), ActionError> {
        if let Some(source) = target.forked_from.as_deref() {
            self.bridge_state
                .apply_thread_fork(source, &target.thread_id)?;
        }
        Ok(())
    }
}

fn fork_source_label(source: &str, forked: bool) -> String {
    if !forked || source.contains("app-server fork") {
        source.to_owned()
    } else {
        format!("{source} (app-server fork)")
    }
}

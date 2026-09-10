use super::{ActionError, ActionExecutor};
use crate::queue_runner::TurnBackend;

impl<B: TurnBackend> ActionExecutor<B> {
    pub async fn control_lock(
        &self,
        thread: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, ActionError> {
        Ok(self.queue.target_lock(thread)?.lock_owned().await)
    }

    /// Shared by command and button. Cache absence means unknown/not controllable,
    /// not proof of idle, and must never trigger a fork or a replacement turn.
    pub async fn verified_control_turn(
        &self,
        channel: u64,
        thread: &str,
        expected: Option<&str>,
    ) -> Result<(String, u64), ActionError> {
        let current = self.target(channel)?.0;
        if current != thread {
            return Err(ActionError::Invalid(
                "control target changed; select the current thread again".into(),
            ));
        }
        self.verified_owned_turn(thread, expected).await
    }

    pub(super) async fn verified_owned_turn(
        &self,
        thread: &str,
        expected: Option<&str>,
    ) -> Result<(String, u64), ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let generation = server.generation();
        let snapshot = server.lifecycle_snapshot().await;
        if !snapshot.healthy || snapshot.quarantined || snapshot.restart_pending {
            return Err(ActionError::Invalid(
                "Codex connection is not ready; active turn is unknown. Retry after reconnect."
                    .into(),
            ));
        }
        let active = server.active_turn_id(thread).await?;
        let Some(turn) = active else {
            return Err(ActionError::Invalid("no currently owned active turn is confirmed; the task may be preparing, finished, or reconnecting. Retry the control when it is active.".into()));
        };
        if expected.is_some_and(|expected| expected != turn) {
            return Err(ActionError::Invalid("the original turn has ended; this button will not steer a later turn. Send a new request.".into()));
        }
        if server.generation() != generation
            || cdr_store::observed_completion::contains(&self.mirror_db, thread, &turn)?
        {
            return Err(ActionError::Invalid(
                "turn completed or connection changed while checking; no control was sent".into(),
            ));
        }
        Ok((turn, generation))
    }
}

//! Bounded read-only reconciliation; original interrupt authority is never reused.
use super::{ActionError, ActionExecutor};
use crate::{
    queue_runner::{QueueRunnerError, TurnBackend},
    settings_binding::SettingsBinding,
};
use cdr_store::ingress::stop::control;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

impl<B: TurnBackend> ActionExecutor<B> {
    pub async fn process_stop_recoveries(&self, cursor: &mut i64) -> Result<usize, ActionError> {
        let candidates = control::recovery_after(&self.mirror_db, *cursor)?;
        if candidates.is_empty() {
            *cursor = 0;
            return Ok(0);
        }
        let mut recovered = 0;
        for (sequence, original) in candidates {
            // Advance before waiting so a slow/busy target cannot starve the next.
            *cursor = sequence;
            let Ok(_guard) = self.queue.target_lock(&original.target)?.try_lock_owned() else {
                continue;
            };
            match tokio::time::timeout(
                Duration::from_millis(1500),
                self.recover_stopped_execution(&original),
            )
            .await
            {
                Ok(Ok(())) => recovered += 1,
                Ok(Err(error)) => eprintln!(
                    "stop_terminal_recovery_held operation={} error={error}",
                    original.operation_id
                ),
                Err(_) => eprintln!(
                    "stop_terminal_recovery_held operation={} read deadline",
                    original.operation_id
                ),
            }
        }
        Ok(recovered)
    }

    async fn recover_stopped_execution(
        &self,
        original: &control::StopControl,
    ) -> Result<(), ActionError> {
        let server = self.server.as_ref().ok_or(ActionError::MissingAppServer)?;
        let permit = self
            .queue
            .admission
            .as_ref()
            .map(crate::restart_readiness::drain::AdmissionGate::try_enter_control)
            .transpose()
            .map_err(QueueRunnerError::from)?;
        let binding: SettingsBinding = serde_json::from_value(original.binding.clone())
            .map_err(|error| ActionError::Invalid(error.to_string()))?;
        let resolver = self.settings_resolver();
        resolver.validate_selected_snapshot(&binding)?;
        let observation = server
            .observe_stopped_execution(&original.target, &original.turn, Duration::from_millis(500))
            .await?;
        let mut db =
            Connection::open_with_flags(&self.mirror_db, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(cdr_store::StoreError::from)?;
        db.busy_timeout(Duration::from_millis(100))
            .map_err(cdr_store::StoreError::from)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(cdr_store::StoreError::from)?;
        let cancelled = control::recover_terminal_in(
            &tx,
            original,
            observation.observation(),
            observation.resident_id(),
            i64::try_from(observation.generation()).map_err(|_| ActionError::IntegerRange)?,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64(),
        )?;
        resolver.validate_selected_snapshot(&binding)?;
        let commit = || {
            observation.with_current_connection(|| tx.commit().map_err(cdr_store::StoreError::from))
        };
        let committed = if let Some(permit) = permit {
            permit
                .with_unsealed(commit)
                .map_err(QueueRunnerError::from)?
        } else {
            commit()
        };
        committed??;
        eprintln!(
            "stop_terminal_recovered operation={} target={} cancelled={cancelled} replay=false",
            original.operation_id, original.target
        );
        Ok(())
    }
}

use super::RecoveryReport;
use crate::queue_runner::{
    BackendFailure, QueueCoordinator, QueueRunnerError, TurnBackend, generation_i64,
};
use std::time::Duration;

impl<B: TurnBackend> crate::queue_runner::TargetLease<'_, B> {
    /// The scheduler already owns the exact target mutex. Never reacquire it or mutate the queue.
    pub(crate) async fn reconcile_orphan_history(&self) -> Result<(), QueueRunnerError> {
        let _ = self
            .queue
            .observe_async_history_inner(self.target())
            .await?;
        Ok(())
    }
}

impl<B: TurnBackend> QueueCoordinator<B> {
    // Called only inside the coordinator's original target lock.
    pub(super) async fn observe_async_history_locked(
        &self,
        target: &str,
        report: &mut RecoveryReport,
    ) {
        match self.observe_async_history_inner(target).await {
            Ok(count) => report.unresolved += count,
            Err(error) => {
                report.unresolved += 1;
                report.read_unavailable_targets.insert(target.into());
                report.unavailable_targets.insert(target.into());
                eprintln!("rust_async_history_review_held target={target} error={error}");
            }
        }
    }

    async fn observe_async_history_inner(&self, target: &str) -> Result<usize, QueueRunnerError> {
        let _permit = self
            .admission
            .as_ref()
            .map(crate::restart_readiness::drain::AdmissionGate::try_enter_control)
            .transpose()?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let Some(observer) = self.backend.resident_instance_id().map(str::to_owned) else {
            return Ok(0);
        };
        let generation = self.backend.generation();
        let Some(snapshot) =
            cdr_store::async_resolution::capture_history_snapshot(&self.db_path, target)?
        else {
            return Ok(0);
        };
        let originals = snapshot.turn_ids();
        let history = tokio::time::timeout_at(
            deadline,
            self.backend.read_async_history(target, &originals),
        )
        .await
        .map_err(|_| BackendFailure::definite("historical review read budget expired"))??;
        let Some(history) = history else {
            return Ok(snapshot.obligation_count());
        };
        if self.backend.resident_instance_id() != Some(observer.as_str())
            || self.backend.generation() != generation
        {
            return Err(BackendFailure::definite(
                "historical review connection changed; candidate not committed",
            )
            .into());
        }
        let _ = cdr_store::async_resolution::retain_history_candidate(
            &self.db_path,
            &snapshot,
            &history,
            &observer,
            generation_i64(generation)?,
        )?;
        if self.admission.is_none() {
            return Ok(snapshot.obligation_count());
        }
        let Some(terminal) =
            cdr_store::async_resolution::capture_terminal_history_snapshot(&self.db_path, target)?
        else {
            return Ok(snapshot.obligation_count());
        };
        let owners = terminal.turn_ids();
        let observation =
            tokio::time::timeout_at(deadline, self.backend.read_async_terminal(target, &owners))
                .await
                .map_err(|_| {
                    BackendFailure::definite("historical terminal read budget expired")
                })??;
        let Some(observation) = observation else {
            return Ok(snapshot.obligation_count());
        };
        if self.backend.resident_instance_id() != Some(observer.as_str())
            || self.backend.generation() != generation
        {
            return Err(BackendFailure::definite(
                "historical terminal connection changed; no settlement",
            )
            .into());
        }
        let settled = cdr_store::async_resolution::settle_terminal_history(
            &self.db_path,
            &terminal,
            &observation,
            &observer,
            generation_i64(generation)?,
        )?;
        Ok(snapshot.obligation_count().saturating_sub(settled))
    }
}

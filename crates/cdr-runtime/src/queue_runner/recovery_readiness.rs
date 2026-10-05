//! Real read-only prerequisites under the queue's target/control/native lifetime.
//! This stage grants no disposition, release, ingress exception or execution.
use super::{BackendFailure, QueueCoordinator, QueueRunnerError, TurnBackend, generation_i64};
use crate::restart_readiness::drain::AdmissionPermit;
use cdr_app_server::NativeRecoveryObservation;
use cdr_store::async_resolution::abandonment::readiness;
use std::time::Duration;
use tokio::{sync::OwnedMutexGuard, time::Instant};

pub struct RecoveryReadiness<'a, B: TurnBackend> {
    queue: &'a QueueCoordinator<B>,
    snapshot: readiness::Snapshot,
    report: readiness::Report,
    observation: NativeRecoveryObservation,
    control: AdmissionPermit,
    deadline: Instant,
    _target: OwnedMutexGuard<()>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    /// Require a real maintenance gate; do not use the legacy optional fallback.
    /// No target lock, connection or transaction is retained while a user decides.
    pub async fn acquire_recovery_readiness(
        &self,
        target: &str,
        disposition: &str,
        revision: i64,
    ) -> Result<RecoveryReadiness<'_, B>, QueueRunnerError> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let control = self
            .admission
            .as_ref()
            .ok_or_else(|| held("recovery requires an admission gate"))?
            .try_enter_control()?;
        control.with_unsealed(|| ())?;
        let target_guard =
            tokio::time::timeout_at(deadline, self.target_lock(target)?.lock_owned())
                .await
                .map_err(|_| held("recovery target lock budget expired"))?;
        control.with_unsealed(|| ())?;
        let snapshot = readiness::capture(&self.db_path, disposition, revision)?;
        if snapshot.thread_id() != target {
            return Err(held("recovery target differs from exact disposition"));
        }
        let owners = snapshot.turn_ids();
        let observation = tokio::time::timeout_at(
            deadline,
            self.backend.read_recovery_prerequisites(target, &owners),
        )
        .await
        .map_err(|_| held("recovery native read budget expired"))??;
        require_backend(self, &observation)?;
        let report = readiness::verify_observation(
            &self.db_path,
            &snapshot,
            observation.observation(),
            observation.resident_id(),
            generation_i64(observation.generation())?,
        )?;
        control
            .with_unsealed(|| observation.check_current())?
            .map_err(|error| held(&error.to_string()))?;
        if Instant::now() >= deadline {
            return Err(held("recovery prerequisite budget expired"));
        }
        Ok(RecoveryReadiness {
            queue: self,
            snapshot,
            report,
            observation,
            control,
            deadline,
            _target: target_guard,
        })
    }
}

impl<B: TurnBackend> RecoveryReadiness<'_, B> {
    #[must_use]
    pub fn report(&self) -> &readiness::Report {
        &self.report
    }
    #[must_use]
    pub fn snapshot(&self) -> &readiness::Snapshot {
        &self.snapshot
    }

    /// Connection/control publication fence only, consumed once. The authenticated
    /// release caller must first revalidate its exact consent, original controls,
    /// local snapshot and final writes inside its already-held DB transaction.
    /// Only that transaction's bounded final commit belongs in this callback.
    /// This API does not create release authority or bypass any existing hold.
    pub fn with_current_connection<R>(
        self,
        publish: impl FnOnce() -> R,
    ) -> Result<R, QueueRunnerError> {
        require_backend(self.queue, &self.observation)?;
        self.control.with_unsealed(|| {
            if Instant::now() >= self.deadline {
                return Err(held("recovery publication budget expired"));
            }
            self.observation
                .with_current_connection(publish)
                .map_err(|error| held(&error.to_string()))
        })?
    }
}

fn require_backend<B: TurnBackend>(
    queue: &QueueCoordinator<B>,
    observation: &NativeRecoveryObservation,
) -> Result<(), QueueRunnerError> {
    if queue.backend.resident_instance_id() != Some(observation.resident_id())
        || queue.backend.generation() != observation.generation()
    {
        return Err(held(
            "recovery observation does not belong to the current queue backend",
        ));
    }
    Ok(())
}

fn held(reason: &str) -> QueueRunnerError {
    BackendFailure::definite(reason).into()
}

#[cfg(test)]
mod tests;

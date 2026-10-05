use crate::queue_runner::{BackendFailure, QueueCoordinator, QueueRunnerError, TurnBackend};
use std::time::Duration;

impl<B: TurnBackend> QueueCoordinator<B> {
    /// Bootstrap uses the same target lock and control admission as runtime work.
    /// On failure the store's exact-incident fallback still denies admission/RPCs.
    pub(crate) async fn install_reviewed_recovery_policy(&self) -> Result<(), QueueRunnerError> {
        let gate = self.admission.as_ref().ok_or_else(|| {
            BackendFailure::definite(
                "recovery policy installation requires the owning runtime control gate",
            )
        })?;
        let lock = self.target_lock(cdr_store::async_resolution::REVIEWED_INCIDENT_THREAD)?;
        let _guard = tokio::time::timeout(Duration::from_secs(3), lock.lock())
            .await
            .map_err(|_| {
                BackendFailure::definite("recovery policy target lock is busy; target remains held")
            })?;
        let _permit = gate.try_enter_control()?;
        cdr_store::async_resolution::install_reviewed_policy(&self.db_path)?;
        Ok(())
    }
}

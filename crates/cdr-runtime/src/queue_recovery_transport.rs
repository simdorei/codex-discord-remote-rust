use cdr_app_server::{AppServerError, ResidentAppServer, ResidentLifecycleSnapshot};

pub(crate) trait QueueRecoveryServer {
    async fn recovery_snapshot(&self) -> ResidentLifecycleSnapshot;
    async fn force_recovery_restart(&self) -> Result<bool, AppServerError>;
}

impl QueueRecoveryServer for ResidentAppServer {
    async fn recovery_snapshot(&self) -> ResidentLifecycleSnapshot {
        self.lifecycle_snapshot().await
    }

    async fn force_recovery_restart(&self) -> Result<bool, AppServerError> {
        self.force_restart_if_quiescent().await
    }
}

pub(crate) async fn stabilize_after_queue_recovery(
    server: &impl QueueRecoveryServer,
) -> Result<bool, AppServerError> {
    let snapshot = server.recovery_snapshot().await;
    if !snapshot.quarantined {
        return Ok(false);
    }
    if server.force_recovery_restart().await? {
        return Ok(true);
    }
    Err(AppServerError::GenerationQuarantined {
        generation: snapshot.generation,
    })
}

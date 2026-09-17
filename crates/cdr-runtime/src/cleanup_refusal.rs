//! Shared notification boundary for an already persisted, known cleanup refusal.
use crate::{action_executor::ActionError, mirror_sync::MirrorSyncError};
use cdr_store::ingress::CleanupRefusal;
use std::path::Path;

pub(crate) fn from_action_error(error: &ActionError) -> Option<CleanupRefusal> {
    match error {
        ActionError::MirrorSync(MirrorSyncError::CleanupProtected { channel, reason }) => {
            let refusal = CleanupRefusal {
                room: *channel,
                reason: (*reason).into(),
            };
            CleanupRefusal::from_outcome(Some(&refusal.outcome()))
        }
        _ => None,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("known mirror refusal saved; {stage}: {source}; hold persistence: {hold_status}")]
pub struct NotificationFailure {
    stage: &'static str,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
    hold_status: String,
}

impl NotificationFailure {
    pub(crate) fn delivery(
        db: &Path,
        key: &str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::record(db, key, "notification delivery unconfirmed", error)
    }

    pub(crate) fn confirmation(
        db: &Path,
        key: &str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::record(
            db,
            key,
            "notification delivery confirmed; ingress confirmation write failed",
            error,
        )
    }

    fn record(
        db: &Path,
        key: &str,
        stage: &'static str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        let hold_status = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(cdr_store::StoreError::from)
            .and_then(|time| cdr_store::ingress::hold(db, key, stage, false, time.as_secs_f64()))
            .map_or_else(|error| error.to_string(), |()| "saved".into());
        Self {
            stage,
            source: Box::new(error),
            hold_status,
        }
    }
}

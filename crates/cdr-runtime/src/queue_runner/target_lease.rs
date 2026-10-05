//! A non-waiting admission owns the same target mutex used by normal queue work.
use super::{QueueCoordinator, QueueRunnerError, TurnBackend};
use tokio::sync::OwnedMutexGuard;

pub(crate) struct TargetLease<'a, B: TurnBackend> {
    pub(super) queue: &'a QueueCoordinator<B>,
    target: String,
    _guard: OwnedMutexGuard<()>,
}

impl<B: TurnBackend> QueueCoordinator<B> {
    pub(crate) fn try_target_lease(
        &self,
        target: &str,
    ) -> Result<Option<TargetLease<'_, B>>, QueueRunnerError> {
        let Ok(guard) = self.target_lock(target)?.try_lock_owned() else {
            return Ok(None);
        };
        Ok(Some(TargetLease {
            queue: self,
            target: target.to_owned(),
            _guard: guard,
        }))
    }
}

impl<B: TurnBackend> TargetLease<'_, B> {
    pub(crate) fn target(&self) -> &str {
        &self.target
    }
    pub(super) fn require_target(&self, target: &str) -> Result<(), QueueRunnerError> {
        if self.target != target {
            return Err(cdr_store::StoreError::InvalidQueueState(
                "queue target lease mismatch".into(),
            )
            .into());
        }
        Ok(())
    }
}

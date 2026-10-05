use super::{MirrorSyncError, MirrorSynchronizer};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, timeout, timeout_at};

const LOCK_DEADLINE: Duration = Duration::from_secs(10);
const OPERATION_DEADLINE: Duration = Duration::from_mins(2);

impl MirrorSynchronizer {
    /// Cooperative deadline; synchronous OS/SQLite work is not forcibly terminated.
    pub(super) async fn with_operation_deadline<T>(
        &self,
        operation: &'static str,
        read_only: bool,
        future: impl Future<Output = Result<T, MirrorSyncError>>,
    ) -> Result<T, MirrorSyncError> {
        let deadline = Instant::now() + OPERATION_DEADLINE;
        let _guard = timeout(LOCK_DEADLINE, self.lock.lock()).await.map_err(|_| {
            MirrorSyncError::Invalid(format!(
                "phase=lock_wait; operation={operation}; deadline={}s; not started; no requests dispatched by this call; current lock owner was not cancelled",
                LOCK_DEADLINE.as_secs(),
            ))
        })?;
        // One budget for all awaits, including the preceding mutex wait.
        // Dropping an expired operation preserves its existing durable custody.
        timeout_at(deadline, future).await.map_err(|_| {
            let outcome = if read_only {
                "read-only inspection incomplete; no mutations dispatched"
            } else {
                "earlier changes may have completed; in-flight remote outcome unconfirmed; no automatic retry issued"
            };
            MirrorSyncError::Invalid(format!(
                "phase=operation; operation={operation}; deadline={}s total including lock wait; {outcome}",
                OPERATION_DEADLINE.as_secs(),
            ))
        })?
    }
}

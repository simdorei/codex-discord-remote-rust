use std::path::{Path, PathBuf};

/// Only the winner of `begin_thread_start` owns this guard. Cancellation must
/// preserve an unconfirmed creation attempt, never authorize another start.
pub(super) struct AttemptGuard {
    db: PathBuf,
    key: String,
}

impl AttemptGuard {
    pub(super) fn new(db: &Path, key: &str) -> Self {
        Self {
            db: db.to_owned(),
            key: key.to_owned(),
        }
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        let recorded = (|| -> Result<(), super::ActionError> {
            if let Some(saved) = cdr_store::ingress::get(&self.db, &self.key)?
                && saved.state == "executing"
                && saved.owner_id.is_none()
            {
                cdr_store::ingress::hold(
                    &self.db,
                    &self.key,
                    "new-thread attempt ended before durable prompt ownership; automatic recreation is disabled",
                    false,
                    super::unix_now()?,
                )?;
            }
            Ok(())
        })();
        if let Err(error) = recorded {
            eprintln!(
                "new_thread_attempt_hold_error request_id={} error={error}",
                self.key
            );
        }
    }
}

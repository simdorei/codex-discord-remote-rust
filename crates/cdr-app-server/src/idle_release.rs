//! A journal-backed, exact-owner capability. No method-name timeout bypass.
use crate::AppServerError;
use std::sync::Arc;

pub(crate) mod gate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdleReleaseToken {
    pub intent_id: String,
    pub owner_id: String,
    pub generation: u64,
    pub thread_id: String,
    pub turn_id: String,
    pub job_id: String,
    pub revision: i64,
    pub state: String,
    pub detail: String,
}

/// Implementations must compare every identity field and commit before returning.
/// They may not resolve uncertain effects from elapsed time or a new owner UUID.
pub trait IdleReleaseJournal: Send + Sync {
    fn before_mutation(
        &self,
        owner: &str,
        generation: u64,
        thread: &str,
    ) -> Result<Option<IdleReleaseToken>, AppServerError>;
    fn check_mutation(&self, thread: &str) -> Result<(), AppServerError>;
    fn resume_required(&self, thread: &str) -> Result<bool, AppServerError>;
    fn verify(&self, token: &IdleReleaseToken, require_idle: bool) -> Result<(), AppServerError>;
    fn transition(
        &self,
        token: &IdleReleaseToken,
        state: &str,
        detail: &str,
    ) -> Result<IdleReleaseToken, AppServerError>;
    fn old_child_exited(&self, owner: &str, generation: u64) -> Result<(), AppServerError>;
}

pub(crate) fn held(message: impl Into<String>) -> AppServerError {
    AppServerError::IdleRelease {
        message: message.into(),
    }
}

pub(crate) struct ExclusivePermit {
    pub(crate) gate: Arc<gate::TargetGate>,
    pub(crate) thread: String,
    pub(crate) journal: Arc<dyn IdleReleaseJournal>,
}

impl Drop for ExclusivePermit {
    fn drop(&mut self) {
        self.gate.release_exclusive(&self.thread);
    }
}

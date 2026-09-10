use cdr_codex_state::CodexStateError;
use cdr_store::StoreError;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionMirrorPoll {
    pub targets: usize,
    pub events: usize,
    pub sent: usize,
}

#[derive(Debug, Error)]
pub enum SessionMirrorError {
    #[error(transparent)]
    State(#[from] CodexStateError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("session mirror Discord delivery failed: {0}")]
    Delivery(String),
    #[error("session mirror cursor is outside the signed integer range")]
    CursorRange,
    #[error("system clock is before the Unix epoch: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("session mirror file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("mapped Codex thread is unavailable")]
    TargetUnavailable,
    #[error("mapped Codex thread rollout file is unavailable")]
    TargetRolloutUnavailable,
    #[error("bot turn ownership is still being recorded; mirror cursor retained for retry")]
    OwnershipPending,
    #[error(
        "session mirror poll failed for {failed_targets} target(s) after attempting all targets; first target {first_target}: {first_error}; all failures: {failure_summary}"
    )]
    TargetBatch {
        failed_targets: usize,
        delivery_failures: usize,
        progress: SessionMirrorPoll,
        first_target: String,
        #[source]
        first_error: Box<SessionMirrorError>,
        failure_summary: String,
    },
}

impl SessionMirrorError {
    #[must_use]
    pub fn is_delivery_failure(&self) -> bool {
        match self {
            Self::Delivery(_) => true,
            Self::TargetBatch {
                delivery_failures, ..
            } => *delivery_failures > 0,
            _ => false,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct TargetFailures {
    failures: Vec<(String, SessionMirrorError)>,
    delivery_failures: usize,
}

impl TargetFailures {
    pub(crate) fn record(&mut self, target: String, error: SessionMirrorError) {
        if error.is_delivery_failure() {
            self.delivery_failures += 1;
        }
        self.failures.push((target, error));
    }

    pub(crate) fn finish(
        mut self,
        progress: SessionMirrorPoll,
    ) -> Result<SessionMirrorPoll, SessionMirrorError> {
        if self.failures.is_empty() {
            return Ok(progress);
        }
        self.failures
            .sort_by(|(left, _), (right, _)| left.cmp(right));
        let failure_summary = self
            .failures
            .iter()
            .map(|(target, error)| format!("target {target}: {error}"))
            .collect::<Vec<_>>()
            .join(" | ");
        let failed_targets = self.failures.len();
        let (first_target, first_error) = self.failures.into_iter().next().expect("not empty");
        Err(SessionMirrorError::TargetBatch {
            failed_targets,
            delivery_failures: self.delivery_failures,
            progress,
            first_target,
            first_error: Box::new(first_error),
            failure_summary,
        })
    }
}

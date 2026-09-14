use std::future::Future;
use std::pin::Pin;

use crate::restart_readiness::drain::DrainGateError;
use cdr_app_server::outcomes::TurnStatus;
use cdr_store::StoreError;
use cdr_store::queue::{
    AppServerForkHandoffError, STARTING_CANDIDATE_HOLD_PREFIX, UNRESOLVED_FORK_ERROR_PREFIX,
};
use thiserror::Error;

pub type BoxBackendFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, BackendFailure>> + Send + 'a>>;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("{message}")]
pub struct BackendFailure {
    pub message: String,
    pub ambiguous: bool,
    pub kind: BackendFailureKind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BackendFailureKind {
    #[default]
    Other,
    ActiveWriter,
    Quarantined,
    ForkFenced,
    StartingCandidatesHeld,
}

impl BackendFailure {
    #[must_use]
    pub fn definite(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: false,
            kind: BackendFailureKind::Other,
        }
    }

    #[must_use]
    pub fn ambiguous(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: true,
            kind: BackendFailureKind::Other,
        }
    }

    #[must_use]
    pub fn active_writer(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: false,
            kind: BackendFailureKind::ActiveWriter,
        }
    }

    #[must_use]
    pub fn quarantined(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: true,
            kind: BackendFailureKind::Quarantined,
        }
    }

    #[must_use]
    pub fn fork_fenced(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: true,
            kind: BackendFailureKind::ForkFenced,
        }
    }

    #[must_use]
    pub fn starting_candidates_held(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ambiguous: true,
            kind: BackendFailureKind::StartingCandidatesHeld,
        }
    }

    #[must_use]
    pub fn persisted(message: impl Into<String>, ambiguous: bool) -> Self {
        let message = message.into();
        let kind = if message.starts_with(UNRESOLVED_FORK_ERROR_PREFIX) {
            BackendFailureKind::ForkFenced
        } else if message.starts_with(STARTING_CANDIDATE_HOLD_PREFIX) {
            BackendFailureKind::StartingCandidatesHeld
        } else if !ambiguous && is_active_writer_message(&message) {
            BackendFailureKind::ActiveWriter
        } else {
            BackendFailureKind::Other
        };
        Self {
            message,
            ambiguous,
            kind,
        }
    }
}

#[must_use]
pub fn is_active_writer_message(message: &str) -> bool {
    message.contains("thread/resume") && message.contains("already has an active writer")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnRecord {
    pub turn_id: String,
    pub status: TurnStatus,
}

pub trait TurnBackend: Send + Sync + 'static {
    fn generation(&self) -> u64;
    /// Only a resident backend can own a subscription; other backends do not release.
    fn resident_instance_id(&self) -> Option<&str> {
        None
    }
    /// A successful thread/start on this exact generation has an empty baseline.
    fn remember_new_thread(&self, _thread_id: &str, _generation: u64) {}
    fn requires_app_server_fork(&self) -> bool {
        false
    }
    fn active_turn_id<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>>;
    fn read_turns<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>>;
    fn resume_thread<'a>(&'a self, thread_id: &'a str) -> BoxBackendFuture<'a, ()>;
    fn fork_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async {
            Err(BackendFailure::definite(
                "thread/fork is not supported by this backend",
            ))
        })
    }
    fn start_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
    ) -> BoxBackendFuture<'a, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submission {
    pub job_id: String,
    pub queued: bool,
    pub turn_id: Option<String>,
    pub warning: Option<BackendFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusyStatus {
    pub busy: bool,
    pub allow_steer: bool,
}

#[derive(Debug, Error)]
pub enum QueueRunnerError {
    #[error(transparent)]
    RestartDrain(#[from] DrainGateError),
    #[error("durable queue operation failed: {0}")]
    Store(#[from] StoreError),
    #[error("Codex turn backend failed: {0}")]
    Backend(#[from] BackendFailure),
    #[error(transparent)]
    ForkHandoff(#[from] AppServerForkHandoffError),
    #[error(
        "app-server thread/fork for {source_thread_id} failed after durable handoff {handoff_id}: {failure}"
    )]
    ForkBackend {
        source_thread_id: String,
        handoff_id: String,
        failure: BackendFailure,
    },
    #[error(
        "app-server thread/fork for {source_thread_id} failed after durable handoff {handoff_id}: {failure}; definite-failure fence cleanup also failed: {cancellation}"
    )]
    ForkCancellation {
        source_thread_id: String,
        handoff_id: String,
        failure: BackendFailure,
        cancellation: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server thread/fork for {source_thread_id} failed after durable handoff {handoff_id}: {failure}; definite-failure fence cleanup also failed: {cancellation}; recording the combined cancellation failure also failed: {recording}"
    )]
    ForkCancellationFailureRecording {
        source_thread_id: String,
        handoff_id: String,
        failure: BackendFailure,
        cancellation: Box<AppServerForkHandoffError>,
        recording: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server thread/fork for {source_thread_id} failed after durable handoff {handoff_id}: {failure}; recording the fork failure also failed: {recording}"
    )]
    ForkFailureRecording {
        source_thread_id: String,
        handoff_id: String,
        failure: BackendFailure,
        recording: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server thread/fork for {source_thread_id} returned target {target_thread_id}, but could not durably stage it for handoff {handoff_id}: {staging}"
    )]
    ForkTargetStage {
        source_thread_id: String,
        handoff_id: String,
        target_thread_id: String,
        staging: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server fork target {target_thread_id} for {source_thread_id} could not be finalized from durable handoff {handoff_id}: {failure}"
    )]
    ForkFinalize {
        source_thread_id: String,
        handoff_id: String,
        target_thread_id: String,
        failure: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server fork target {target_thread_id} for {source_thread_id} could not be finalized from durable handoff {handoff_id}: {failure}; recording that finalization failure also failed: {recording}"
    )]
    ForkFinalizeRecording {
        source_thread_id: String,
        handoff_id: String,
        target_thread_id: String,
        failure: Box<AppServerForkHandoffError>,
        recording: Box<AppServerForkHandoffError>,
    },
    #[error(
        "app-server fork handoff {handoff_id} for {source_thread_id} is unresolved; automatic retry is blocked to prevent a duplicate fork; last fork error: {last_fork_error}"
    )]
    UnresolvedForkHandoff {
        source_thread_id: String,
        handoff_id: String,
        last_fork_error: String,
    },
    #[error(
        "durable queue attempt ownership changed for job {job_id} after backend start observation {observed_turn_id:?}; automatic replay is blocked"
    )]
    AttemptClaimLost {
        job_id: String,
        observed_turn_id: Option<String>,
    },
    #[error("completed app-server fork targets form a cycle from {source_thread_id}")]
    ForkTargetCycle { source_thread_id: String },
    #[error("runtime queue lock was poisoned")]
    LockPoisoned,
    #[error("Discord or app-server generation does not fit the SQLite integer contract")]
    IntegerRange,
    #[error("system clock is before the Unix epoch: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
}

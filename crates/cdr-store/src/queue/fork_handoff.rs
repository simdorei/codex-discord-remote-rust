mod cancellation_failure;
mod definite_failure;
mod failure;
mod read;
mod retirement;
pub use retirement::retire_copy_only_handoffs;
mod storage;
mod target;
mod transition;
mod unresolved_notice;

pub use cancellation_failure::record_app_server_fork_cancellation_failure;
pub use definite_failure::{
    DEFINITE_FORK_ERROR_PREFIX, RecordedDefiniteAppServerForkFailure,
    record_and_cancel_app_server_fork_handoff_after_definite_failure,
    repair_legacy_definite_app_server_fork_failures,
};
pub use failure::{
    UNRESOLVED_FORK_ERROR_PREFIX, record_app_server_fork_failure,
    record_app_server_fork_finalize_failure,
};
pub(crate) use read::canonical_completed_target;
pub use read::{
    completed_app_server_fork_target_for_source, is_app_server_managed_target,
    unresolved_app_server_fork_handoff_for_source,
};
pub(super) use read::{ensure_no_unresolved_handoff, ensure_schema, ensure_source_not_moved};
pub use target::{
    cancel_app_server_fork_handoff_after_definite_failure, complete_app_server_fork_handoff,
    finalize_app_server_fork_handoff, stage_app_server_fork_target,
};

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::TransactionBehavior;
use thiserror::Error;

use super::StoredQueueJob;
use crate::StoreError;
use crate::schema::open_initialized;
use transition::{
    conflicting_or_existing, mapping_snapshot, validate_begin, validate_no_other_inflight,
    validate_starting_job,
};

pub const STARTING_ATTEMPT_LEASE_SECONDS: f64 = 120.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NewAppServerForkHandoff<'a> {
    pub handoff_id: &'a str,
    pub ambiguous_job_id: Option<&'a str>,
    pub source_thread_id: &'a str,
    pub expected_generation: i64,
    pub quarantine_reason: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerForkHandoff {
    pub handoff_id: String,
    pub ambiguous_job_id: Option<String>,
    pub source_thread_id: String,
    pub expected_generation: i64,
    pub discord_channel_id: i64,
    pub discord_thread_id: i64,
    pub quarantine_reason: String,
    pub last_fork_error: String,
    pub fork_failure_ambiguous: bool,
    pub observed_target_thread_id: Option<String>,
    pub target_thread_id: Option<String>,
    pub completed_generation: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BegunAppServerForkHandoff {
    pub handoff: AppServerForkHandoff,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletedAppServerForkHandoff {
    pub handoff: AppServerForkHandoff,
    pub quarantined_job: Option<StoredQueueJob>,
    pub retargeted_jobs: Vec<StoredQueueJob>,
    pub applied: bool,
}

#[derive(Debug, Error)]
pub enum AppServerForkHandoffError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("invalid app-server fork handoff identity")]
    InvalidIdentity,
    #[error("a different fork handoff already fences source thread {source_thread_id}")]
    ConflictingIntent { source_thread_id: String },
    #[error("ambiguous starting job changed before fork handoff: {job_id}")]
    StaleStartingJob { job_id: String },
    #[error("mirror mapping is missing, stale, or duplicated for {source_thread_id}")]
    MissingOrStaleMapping { source_thread_id: String },
    #[error("fork target is already in use: {target_thread_id}")]
    TargetConflict { target_thread_id: String },
    #[error("fork target has not been durably observed for handoff {handoff_id}")]
    ForkTargetNotObserved { handoff_id: String },
    #[error("fork target {target_thread_id} was already observed for handoff {handoff_id}")]
    ForkTargetAlreadyObserved {
        handoff_id: String,
        target_thread_id: String,
    },
    #[error("source has another starting or running queue job: {source_thread_id}")]
    AdditionalInFlight { source_thread_id: String },
    #[error("starting attempt lease is still active: {job_id}")]
    StartingAttemptLeaseActive { job_id: String },
    #[error("ambiguous fork handoff cannot be cancelled: {handoff_id}")]
    AmbiguousForkCannotBeCancelled { handoff_id: String },
}

impl From<rusqlite::Error> for AppServerForkHandoffError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Store(StoreError::Database(error))
    }
}

pub fn begin_app_server_fork_handoff(
    path: &Path,
    request: NewAppServerForkHandoff<'_>,
) -> Result<BegunAppServerForkHandoff, AppServerForkHandoffError> {
    validate_begin(&request)?;
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::dead_generation::ensure_target_available(&transaction, request.source_thread_id)?;
    storage::ensure_table(&transaction)?;
    if let Some(existing) = conflicting_or_existing(&transaction, &request)? {
        transaction.commit()?;
        return Ok(BegunAppServerForkHandoff {
            handoff: existing,
            created: false,
        });
    }
    let observed_at = now()?;
    validate_starting_job(&transaction, &request, Some(observed_at))?;
    validate_no_other_inflight(
        &transaction,
        request.source_thread_id,
        request.ambiguous_job_id,
    )?;
    let (discord_channel_id, discord_thread_id) =
        mapping_snapshot(&transaction, request.source_thread_id)?;
    let handoff = AppServerForkHandoff {
        handoff_id: request.handoff_id.to_owned(),
        ambiguous_job_id: request.ambiguous_job_id.map(str::to_owned),
        source_thread_id: request.source_thread_id.to_owned(),
        expected_generation: request.expected_generation,
        discord_channel_id,
        discord_thread_id,
        quarantine_reason: bounded_reason(request.quarantine_reason),
        last_fork_error: String::new(),
        fork_failure_ambiguous: false,
        observed_target_thread_id: None,
        target_thread_id: None,
        completed_generation: None,
    };
    storage::insert(&transaction, &handoff, observed_at)?;
    transaction.commit()?;
    Ok(BegunAppServerForkHandoff {
        handoff,
        created: true,
    })
}

fn now() -> Result<f64, AppServerForkHandoffError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::from)?
        .as_secs_f64())
}

fn bounded_reason(reason: &str) -> String {
    let reason = reason.trim();
    let reason = if reason.is_empty() {
        "ambiguous app-server fork handoff"
    } else {
        reason
    };
    reason.chars().take(900).collect()
}

fn bounded_fork_error(error: &str) -> String {
    let error = error.trim();
    let error = if error.is_empty() {
        "app-server fork failed without an error message"
    } else {
        error
    };
    error.chars().take(1_000).collect()
}

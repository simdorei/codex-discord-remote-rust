mod cancel_ingress;
mod cancel_pending;
mod cancellation_schema;
mod failure;
pub use cancel_pending::{cancel_latest_pending, cancel_latest_pending_on_route};
pub(crate) use cancellation_schema::{cancellation_schema_current, migrate_cancellation_schema};
mod fork_handoff;
mod generation;
mod goal;
mod managed_target;
pub(crate) mod read;
mod starting_hold;
mod write;

pub use failure::record_preflight_failure;
pub(crate) use fork_handoff::canonical_completed_target;
pub use fork_handoff::{
    AppServerForkHandoff, AppServerForkHandoffError, BegunAppServerForkHandoff,
    CompletedAppServerForkHandoff, DEFINITE_FORK_ERROR_PREFIX, NewAppServerForkHandoff,
    RecordedDefiniteAppServerForkFailure, STARTING_ATTEMPT_LEASE_SECONDS,
    UNRESOLVED_FORK_ERROR_PREFIX, begin_app_server_fork_handoff,
    cancel_app_server_fork_handoff_after_definite_failure, complete_app_server_fork_handoff,
    completed_app_server_fork_target_for_source, finalize_app_server_fork_handoff,
    is_app_server_managed_target, record_and_cancel_app_server_fork_handoff_after_definite_failure,
    record_app_server_fork_cancellation_failure, record_app_server_fork_failure,
    record_app_server_fork_finalize_failure, repair_legacy_definite_app_server_fork_failures,
    retire_copy_only_handoffs, stage_app_server_fork_target,
    unresolved_app_server_fork_handoff_for_source,
};
pub use generation::{adopt_generation, adopt_target_generation};
pub use goal::{
    attach_goal_turn, attach_goal_turn_if_owned, attach_goal_turn_observed_if_owned,
    mark_goal_waiting,
};
pub use managed_target::mark_app_server_managed_target;
pub(crate) use managed_target::mark_in_transaction as mark_managed_target_in_transaction;
pub(crate) use read::select_job;
pub use read::{list, list_filtered};
pub use starting_hold::{
    STARTING_CANDIDATE_HOLD_PREFIX, hold_starting_for_ambiguous_candidates_if_claimed,
};
pub use write::{
    begin_attempt, complete, discard_for_generation, discard_observed, enqueue,
    enqueue_if_mirror_matches, flush, mark_running, mark_running_if_claimed, record_start_failure,
    record_start_failure_if_claimed, retract, try_begin_attempt,
};
pub(crate) use write::{enqueue_in_transaction, ensure_mirror_matches};

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub enum QueueJobState {
    Pending,
    Starting,
    Running,
    Quarantined,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct StoredQueueJob {
    pub job_id: String,
    pub target_thread_id: String,
    pub channel_id: i64,
    pub owner_user_id: Option<i64>,
    pub discord_message_id: Option<i64>,
    pub app_server_generation: i64,
    pub execution_generation: Option<i64>,
    /// Observation evidence for the currently attached turn, not original-input execution authority.
    pub turn_observation_generation: Option<i64>,
    pub goal_waiting: bool,
    pub prompt: String,
    pub queued: bool,
    pub ack_sent: bool,
    pub state: QueueJobState,
    pub attempt_count: i64,
    pub turn_id: Option<String>,
    pub baseline_turn_ids: Vec<String>,
    pub last_error: String,
    pub created_at: f64,
    pub updated_at: f64,
}

impl StoredQueueJob {
    #[must_use]
    pub fn completion_evidence_generation(&self) -> i64 {
        self.turn_observation_generation
            .unwrap_or(self.app_server_generation)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NewQueueJob<'a> {
    pub job_id: &'a str,
    pub target_thread_id: &'a str,
    pub channel_id: i64,
    pub owner_user_id: Option<i64>,
    pub discord_message_id: Option<i64>,
    pub app_server_generation: i64,
    pub prompt: &'a str,
    pub queued: bool,
    pub ack_sent: bool,
    pub created_at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueEnqueueResult {
    pub job: StoredQueueJob,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExpectedMirrorMapping<'a> {
    pub discord_channel_id: i64,
    pub target_thread_id: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueGenerationAdoption {
    pub jobs: Vec<StoredQueueJob>,
    pub adopted_count: usize,
}

impl QueueJobState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Quarantined => "quarantined",
        }
    }
}

pub(crate) const QUARANTINED_TURN_PREFIX: &str = "cdr-quarantined:";
pub(crate) const QUARANTINED_ERROR_PREFIX: &str = "[cdr-rust:app-server-fork-quarantine:v1] ";

pub(crate) fn is_quarantine_encoding(
    raw_state: &str,
    turn_id: Option<&str>,
    last_error: &str,
) -> bool {
    raw_state == "running"
        && turn_id.is_some_and(|value| value.starts_with(QUARANTINED_TURN_PREFIX))
        && last_error.starts_with(QUARANTINED_ERROR_PREFIX)
}

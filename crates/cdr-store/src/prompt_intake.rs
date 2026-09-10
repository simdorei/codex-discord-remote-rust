mod busy_queue;
mod lease;
mod new_thread;
mod promotion;
mod read;
mod storage;
mod write;

pub use busy_queue::{BusyQueueAdmission, admit_busy_queue};
pub use lease::{
    record_prompt_intake_failure_if_claimed, release_all_prompt_intake_claims,
    renew_prompt_intake_claim_if_current, try_claim_prompt_intake,
};
pub use new_thread::admit_prompt_intake_with_ingress;
pub use new_thread::admit_prompt_intake_with_ingress_and_reply;
pub use promotion::promote_prompt_intake_to_queue;
pub use read::{
    get_prompt_intake, list_prompt_intakes, list_ready_prompt_intakes,
    prompt_intake_has_durable_owner,
};
pub use write::{
    admit_prompt_intake, canonicalize_prompt_intake_target, remove_prompt_intake_if_queued,
};

use rusqlite::{Connection, Transaction, params};

use crate::Result;

#[derive(Clone, Copy, Debug)]
pub struct NewPromptIntake<'a> {
    pub job_id: &'a str,
    pub target_thread_id: &'a str,
    pub channel_id: i64,
    pub owner_user_id: Option<i64>,
    pub discord_message_id: Option<i64>,
    pub raw_prompt: &'a str,
    pub auto_queue_when_busy: bool,
    pub require_current_mirror: bool,
    pub created_at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoredPromptIntake {
    pub job_id: String,
    pub target_thread_id: String,
    pub channel_id: i64,
    pub owner_user_id: Option<i64>,
    pub discord_message_id: Option<i64>,
    pub raw_prompt: String,
    pub auto_queue_when_busy: bool,
    pub require_current_mirror: bool,
    pub attempt_count: i64,
    pub last_error: String,
    pub retry_after: f64,
    pub claim_token: Option<String>,
    pub claim_expires_at: f64,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromptIntakeAdmission {
    pub intake: StoredPromptIntake,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromptIntakeClaim {
    pub intake: StoredPromptIntake,
    pub claim_token: String,
}

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    storage::ensure_schema(connection)
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    storage::schema_current(connection)
}

pub(crate) fn retarget_for_fork(
    transaction: &Transaction<'_>,
    source_thread_id: &str,
    target_thread_id: &str,
    updated_at: f64,
) -> Result<usize> {
    storage::ensure_schema(transaction)?;
    Ok(transaction.execute(
        "UPDATE codex_prompt_intakes SET target_thread_id = ?, \
         last_error = CASE WHEN instr(last_error, ?) = 1 THEN '' ELSE last_error END, \
         updated_at = ? \
         WHERE target_thread_id = ?",
        params![
            target_thread_id,
            crate::queue::UNRESOLVED_FORK_ERROR_PREFIX,
            updated_at,
            source_thread_id
        ],
    )?)
}

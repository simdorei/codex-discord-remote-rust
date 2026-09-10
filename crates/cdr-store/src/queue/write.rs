mod attempt;
mod enqueue;

pub use attempt::{
    begin_attempt, mark_running, mark_running_if_claimed, record_start_failure,
    record_start_failure_if_claimed, try_begin_attempt,
};
pub use enqueue::{enqueue, enqueue_if_mirror_matches};
pub(crate) use enqueue::{enqueue_in_transaction, ensure_mirror_matches};

use std::path::Path;

use rusqlite::{Connection, TransactionBehavior, params};

use super::read::all_jobs;
use super::{QueueJobState, StoredQueueJob};
use crate::Result;
use crate::schema::open_initialized;

pub fn complete(path: &Path, job_id: &str) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM codex_turn_queue WHERE job_id = ? AND NOT EXISTS \
            (SELECT 1 FROM codex_dead_generation_holds hold \
             WHERE hold.target_thread_id = codex_turn_queue.target_thread_id)",
        [job_id],
    )? == 1)
}

pub fn discard_for_generation(
    path: &Path,
    current_generation: Option<i64>,
) -> Result<Vec<StoredQueueJob>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let jobs = all_jobs(&transaction)?
        .into_iter()
        .filter(|job| current_generation.is_none_or(|value| job.app_server_generation != value))
        .collect::<Vec<_>>();
    delete_observed(&transaction, &jobs)?;
    transaction.commit()?;
    Ok(jobs)
}

pub fn discard_observed(path: &Path, observed: &[StoredQueueJob]) -> Result<Vec<StoredQueueJob>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = all_jobs(&transaction)?;
    let removed = current
        .into_iter()
        .filter(|job| {
            observed.iter().any(|item| {
                item.job_id == job.job_id && item.app_server_generation == job.app_server_generation
            })
        })
        .collect::<Vec<_>>();
    delete_observed(&transaction, &removed)?;
    transaction.commit()?;
    Ok(removed)
}

fn delete_observed(connection: &Connection, jobs: &[StoredQueueJob]) -> Result<()> {
    for job in jobs {
        connection.execute(
            "DELETE FROM codex_turn_queue WHERE job_id = ? AND app_server_generation = ?",
            params![job.job_id, job.app_server_generation],
        )?;
    }
    Ok(())
}

pub fn flush(path: &Path, target: &str, generation: i64) -> Result<Vec<StoredQueueJob>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let jobs = all_jobs(&transaction)?
        .into_iter()
        .filter(|job| job.target_thread_id == target && job.app_server_generation == generation)
        .collect::<Vec<_>>();
    delete_observed(&transaction, &jobs)?;
    transaction.commit()?;
    Ok(jobs)
}

pub fn retract(
    path: &Path,
    target: &str,
    channel_id: Option<i64>,
    owner_user_id: Option<i64>,
) -> Result<Option<StoredQueueJob>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    super::fork_handoff::ensure_no_unresolved_handoff(&transaction, target)?;
    super::fork_handoff::ensure_source_not_moved(&transaction, target)?;
    let selected = all_jobs(&transaction)?.into_iter().rev().find(|job| {
        job.target_thread_id == target
            && job.state == QueueJobState::Pending
            && channel_id.is_none_or(|value| job.channel_id == value)
            && owner_user_id.is_none_or(|value| job.owner_user_id == Some(value))
    });
    if let Some(job) = &selected {
        transaction.execute(
            "DELETE FROM codex_turn_queue WHERE job_id = ?",
            [&job.job_id],
        )?;
    }
    transaction.commit()?;
    Ok(selected)
}

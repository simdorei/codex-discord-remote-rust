use std::path::Path;

use rusqlite::{Connection, Row, TransactionBehavior, params};

use crate::mirror::turn_origin_marker;
use crate::queue::select_job;
use crate::schema::open_initialized;
use crate::{Result, StoreError};

const COLUMNS: &str = "delivery_id, job_id, target_thread_id, turn_id, channel_id, \
    content, attempt_count, last_error, created_at, updated_at";

#[derive(Clone, Debug, PartialEq)]
pub struct StoredDelivery {
    pub delivery_id: String,
    pub job_id: String,
    pub target_thread_id: String,
    pub turn_id: String,
    pub channel_id: i64,
    pub content: String,
    pub attempt_count: i64,
    pub last_error: String,
    pub created_at: f64,
    pub updated_at: f64,
}

impl StoredDelivery {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            delivery_id: row.get(0)?,
            job_id: row.get(1)?,
            target_thread_id: row.get(2)?,
            turn_id: row.get(3)?,
            channel_id: row.get(4)?,
            content: row.get(5)?,
            attempt_count: row.get(6)?,
            last_error: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}

pub fn stage_queue_completion(
    path: &Path,
    job_id: &str,
    content: &str,
    now: f64,
) -> Result<StoredDelivery> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let job = select_job(&transaction, job_id)?;
    crate::dead_generation::ensure_target_available(&transaction, &job.target_thread_id)?;
    crate::mirror::record_job_origin(&transaction, &job)?;
    let turn_id = job
        .turn_id
        .as_deref()
        .ok_or_else(|| StoreError::QueueJobHasNoTurn(job_id.into()))?;
    transaction.execute(
        "INSERT OR IGNORE INTO codex_delivery_outbox (delivery_id, job_id, \
         target_thread_id, turn_id, channel_id, content, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            job.job_id,
            job.job_id,
            job.target_thread_id,
            turn_id,
            job.channel_id,
            content,
            now,
            now,
        ],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO codex_session_mirror_events \
         (event_digest, codex_thread_id, created_at) VALUES (?, ?, ?)",
        params![
            turn_origin_marker(&job.target_thread_id, turn_id),
            job.target_thread_id,
            now,
        ],
    )?;
    transaction.execute(
        "DELETE FROM codex_turn_queue WHERE job_id = ?",
        [&job.job_id],
    )?;
    transaction.execute(
        "DELETE FROM codex_observed_completions WHERE thread_id=? AND turn_id=?",
        params![job.target_thread_id, turn_id],
    )?;
    transaction.execute(
        "DELETE FROM codex_observed_final_answers WHERE thread_id=? AND turn_id=?",
        params![job.target_thread_id, turn_id],
    )?;
    let delivery = select(&transaction, &job.job_id)?;
    transaction.commit()?;
    Ok(delivery)
}

pub fn list_pending(path: &Path) -> Result<Vec<StoredDelivery>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {COLUMNS} FROM codex_delivery_outbox ORDER BY created_at, delivery_id"
    ))?;
    let rows = statement
        .query_map([], StoredDelivery::read)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn record_failure(
    path: &Path,
    delivery_id: &str,
    error: &str,
    now: f64,
) -> Result<StoredDelivery> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bounded: String = error.trim().chars().take(1_000).collect();
    if transaction.execute(
        "UPDATE codex_delivery_outbox SET attempt_count = attempt_count + 1, \
         last_error = ?, updated_at = ? WHERE delivery_id = ?",
        params![bounded, now, delivery_id],
    )? != 1
    {
        return Err(StoreError::DeliveryNotFound(delivery_id.into()));
    }
    let delivery = select(&transaction, delivery_id)?;
    transaction.commit()?;
    Ok(delivery)
}

pub fn complete(path: &Path, delivery_id: &str) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM codex_delivery_outbox WHERE delivery_id = ?",
        [delivery_id],
    )? == 1)
}

fn select(connection: &Connection, delivery_id: &str) -> Result<StoredDelivery> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM codex_delivery_outbox WHERE delivery_id = ?"),
            [delivery_id],
            StoredDelivery::read,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                StoreError::DeliveryNotFound(delivery_id.into())
            }
            other => other.into(),
        })
}

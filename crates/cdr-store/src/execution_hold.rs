//! Durable per-request non-replay authority, independent of model selection.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub const PREFIX: &str = "[cdr-rust:execution-held:v1] ";

pub(crate) fn migrate_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS cdr_execution_holds (
        job_id TEXT PRIMARY KEY, target_thread_id TEXT NOT NULL,
        reason TEXT NOT NULL, evidence_json TEXT NOT NULL, created_at REAL NOT NULL
    );",
    )?;
    Ok(())
}

pub(crate) fn schema_current(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT COUNT(*)=5 FROM pragma_table_info('cdr_execution_holds')",
        [],
        |r| r.get(0),
    )?)
}

pub fn reason(path: &Path, job: &str) -> Result<Option<String>> {
    reason_in(&open_initialized(path)?, job)
}

pub fn eligible_jobs(
    path: &Path,
    jobs: Vec<crate::queue::StoredQueueJob>,
) -> Result<Vec<crate::queue::StoredQueueJob>> {
    let db = open_initialized(path)?;
    let mut eligible = Vec::new();
    for job in jobs {
        if job.state != crate::queue::QueueJobState::Pending
            || reason_in(&db, &job.job_id)?.is_none()
        {
            eligible.push(job);
        }
    }
    Ok(eligible)
}

pub(crate) fn reason_in(db: &Connection, job: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT reason FROM cdr_execution_holds WHERE job_id=?",
            [job],
            |r| r.get(0),
        )
        .optional()?)
}

pub(crate) fn require_unheld_in(db: &Connection, job: &str) -> Result<()> {
    if let Some(reason) = reason_in(db, job)? {
        return Err(StoreError::Integrity(format!("{PREFIX}{reason}")));
    }
    Ok(())
}

pub(crate) fn hold_in(
    db: &Connection,
    job: &str,
    target: &str,
    reason: &str,
    evidence: &str,
) -> Result<()> {
    // Holds are never cleared by a settings mutation, queue kick or generation adoption.
    db.execute(
        "INSERT OR IGNORE INTO cdr_execution_holds
        (job_id,target_thread_id,reason,evidence_json,created_at) VALUES (?,?,?,?,unixepoch())",
        params![job, target, reason, evidence],
    )?;
    Ok(())
}

#[must_use]
pub fn legacy_or_current_error(error: &str) -> bool {
    error.starts_with(PREFIX) || error.starts_with(crate::reserve_policy::HOLD_PREFIX)
}

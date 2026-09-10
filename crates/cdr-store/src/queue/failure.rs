use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{TransactionBehavior, params};

use super::StoredQueueJob;
use super::read::select_job;
use crate::Result;
use crate::schema::open_initialized;

pub fn record_preflight_failure(
    path: &Path,
    job_id: &str,
    generation: i64,
    error: &str,
) -> Result<Option<StoredQueueJob>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let bounded_error: String = error.trim().chars().take(1_000).collect();
    let updated = transaction.execute(
        "UPDATE codex_turn_queue SET \
         attempt_count = CASE WHEN attempt_count < 9223372036854775807 \
             THEN attempt_count + 1 ELSE attempt_count END, \
         last_error = ?, updated_at = ? \
         WHERE job_id = ? AND app_server_generation = ? AND state = 'pending'",
        params![bounded_error, now()?, job_id, generation],
    )?;
    let job = (updated == 1)
        .then(|| select_job(&transaction, job_id))
        .transpose()?;
    transaction.commit()?;
    Ok(job)
}

fn now() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

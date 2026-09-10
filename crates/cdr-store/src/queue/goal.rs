use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{TransactionBehavior, params};

use crate::schema::open_initialized;
use crate::{Result, StoreError};

pub fn mark_goal_waiting(
    path: &Path,
    job_id: &str,
    turn_id: &str,
    generation: i64,
) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "UPDATE codex_turn_queue SET goal_waiting = 1, updated_at = ? \
         WHERE job_id = ? AND turn_id = ? AND state = 'running' \
         AND app_server_generation = ? AND NOT EXISTS \
         (SELECT 1 FROM codex_dead_generation_holds hold \
          WHERE hold.target_thread_id = codex_turn_queue.target_thread_id)",
        params![now()?, job_id, turn_id, generation],
    )? == 1)
}

pub fn attach_goal_turn(
    path: &Path,
    target_thread_id: &str,
    turn_id: &str,
    generation: i64,
) -> Result<bool> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if crate::dead_generation::target_is_held_in(&transaction, target_thread_id)? {
        return Ok(false);
    }
    let mut statement = transaction.prepare(
        "SELECT job_id FROM codex_turn_queue WHERE target_thread_id = ? \
         AND state = 'running' AND goal_waiting = 1 AND app_server_generation = ?",
    )?;
    let job_ids = statement
        .query_map(params![target_thread_id, generation], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let Some(job_id) = job_ids.first() else {
        transaction.commit()?;
        return Ok(false);
    };
    if job_ids.len() != 1 {
        return Err(StoreError::InvalidQueueState(format!(
            "multiple goal-waiting jobs for {target_thread_id}"
        )));
    }
    transaction.execute(
        "UPDATE codex_turn_queue SET turn_id = ?, goal_waiting = 0, updated_at = ? WHERE job_id = ?",
        params![turn_id, now()?, job_id],
    )?;
    transaction.commit()?;
    Ok(true)
}

fn now() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

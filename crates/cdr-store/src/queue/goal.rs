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
        "UPDATE codex_turn_queue SET turn_id = ?, turn_observation_generation = ?, goal_waiting = 0, updated_at = ? WHERE job_id = ?",
        params![turn_id, generation, now()?, job_id],
    )?;
    transaction.commit()?;
    Ok(true)
}

/// Attach a freshly observed Goal turn to one exact persisted waiting owner.
/// Historical generations are not rewritten or treated as live RPC authority.
/// The caller validates observation identity separately from this ownership CAS.
pub fn attach_goal_turn_if_owned(
    path: &Path,
    expected: &super::StoredQueueJob,
    turn_id: &str,
) -> Result<bool> {
    // Compatibility path: no claim of observing a different generation.
    attach_goal_turn_observed_if_owned(
        path,
        expected,
        turn_id,
        expected.completion_evidence_generation(),
    )
}

/// Bind turn ID and its actual observed generation in the same owner CAS.
pub fn attach_goal_turn_observed_if_owned(
    path: &Path,
    expected: &super::StoredQueueJob,
    turn_id: &str,
    observation_generation: i64,
) -> Result<bool> {
    if observation_generation < 0
        || expected.state != super::QueueJobState::Running
        || !expected.goal_waiting
        || expected.turn_id.is_none()
        || expected.turn_id.as_deref() == Some(turn_id)
        || turn_id.trim().is_empty()
    {
        return Ok(false);
    }
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if crate::dead_generation::target_is_held_in(&transaction, &expected.target_thread_id)? {
        return Ok(false);
    }
    // Fail closed on duplicate owners across ALL generations, including rows
    // that a current-generation-only query would accidentally hide.
    let mut statement = transaction.prepare(
        "SELECT job_id FROM codex_turn_queue WHERE target_thread_id=? AND state='running'",
    )?;
    let owners = statement
        .query_map([&expected.target_thread_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    if owners.len() > 1 {
        return Err(StoreError::InvalidQueueState(format!(
            "multiple running jobs for {}",
            expected.target_thread_id
        )));
    }
    if owners.first() != Some(&expected.job_id)
        || super::read::select_job(&transaction, &expected.job_id)? != *expected
    {
        return Ok(false);
    }
    // A delayed turn/started for an already handed-off completed turn must not
    // rewind this job after a later Goal turn has finished.
    let completed_origin: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_session_mirror_events \
         WHERE event_digest=? AND codex_thread_id=?)",
        params![
            crate::mirror::turn_origin_marker(&expected.target_thread_id, turn_id),
            expected.target_thread_id
        ],
        |row| row.get(0),
    )?;
    if completed_origin {
        return Ok(false);
    }
    let changed = transaction.execute(
        "UPDATE codex_turn_queue SET turn_id=?, turn_observation_generation=?, goal_waiting=0, updated_at=? WHERE job_id=?",
        params![turn_id, observation_generation, now()?, expected.job_id],
    )?;
    transaction.commit()?;
    Ok(changed == 1)
}

fn now() -> Result<f64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64())
}

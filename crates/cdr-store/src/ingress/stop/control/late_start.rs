//! Attach a late exact start ACK to its already accepted stop, never a new owner.
use super::super::{StopScope, hold_snapshot, revision, validate};
use super::{StopControl, refused, retained_in};
use crate::{
    Result,
    queue::{QueueJobState, StoredQueueJob},
};
use rusqlite::{Connection, params};
use serde_json::Value;

pub(crate) fn bind_in(
    db: &Connection,
    before: &StoredQueueJob,
    running: &StoredQueueJob,
    resident: &str,
) -> Result<()> {
    let Some((operation, scope_text)) = revision::latest_scope_in(db, &before.target_thread_id)?
    else {
        return Ok(());
    };
    let scope: Value = serde_json::from_str(&scope_text)?;
    let jobs = scope
        .get("jobs")
        .and_then(Value::as_array)
        .ok_or_else(refused)?;
    if !jobs
        .iter()
        .any(|id| id.as_str() == Some(before.job_id.as_str()))
    {
        return Ok(());
    }
    // An existing Running stop is not widened or replaced by another ACK.
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_stop_controls WHERE operation_id=?)",
        [&operation],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    if resident.trim().is_empty()
        || !same_original(before, running)
        || jobs.len() > 128
        || scope["target"] != before.target_thread_id
        || scope["channel"].as_i64() != Some(before.channel_id)
        || scope["owner"].as_i64() != before.owner_user_id
    {
        return Err(refused());
    }
    let binding = scope.get("binding").ok_or_else(refused)?;
    validate(
        db,
        StopScope {
            target: &before.target_thread_id,
            channel: before.channel_id,
            owner: before.owner_user_id.ok_or_else(refused)?,
        },
        binding,
        None,
    )?;
    let hold = hold_snapshot(db, &before.job_id)?.ok_or_else(refused)?;
    if hold.0 != before.target_thread_id {
        return Err(refused());
    }
    let turn = running.turn_id.as_deref().ok_or_else(refused)?;
    if turn.is_empty()
        || turn.trim() != turn
        || before.baseline_turn_ids.iter().any(|id| id == turn)
    {
        return Err(refused());
    }
    let control = StopControl {
        operation_id: operation.clone(),
        target: before.target_thread_id.clone(),
        channel: before.channel_id,
        owner: before.owner_user_id.ok_or_else(refused)?,
        resident: resident.into(),
        generation: before.app_server_generation,
        turn: turn.into(),
        binding: binding.clone(),
        jobs: vec![serde_json::to_string(running)?],
        // Only an exact terminal may settle this single original. Missing scope
        // evidence or a queue/intake ID collision must remain unresolved.
        can_settle: single_original_can_settle_in(db, &scope, &before.job_id, jobs.len())?,
    };
    if db.execute(
        "INSERT INTO cdr_stop_controls(operation_id,target_thread_id,resident_owner,
         generation,turn_id,record_json,phase) VALUES(?,?,?,?,?,?,'accepted')",
        params![
            control.operation_id,
            control.target,
            control.resident,
            control.generation,
            control.turn,
            serde_json::to_string(&control)?
        ],
    )? != 1
    {
        return Err(refused());
    }
    let pristine: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_stop_controls WHERE operation_id=?
         AND phase='accepted' AND claim_token IS NULL AND wire_attempt IS NULL
         AND wire_id IS NULL AND terminal_json IS NULL)",
        [&operation],
        |row| row.get(0),
    )?;
    if !pristine
        || !retained_in(db, &control)?
        || hold_snapshot(db, &before.job_id)?.as_ref() != Some(&hold)
        || crate::queue::select_job(db, &before.job_id)? != *running
        || revision::latest_scope_in(db, &before.target_thread_id)? != Some((operation, scope_text))
    {
        return Err(refused());
    }
    Ok(())
}

fn single_original_can_settle_in(
    db: &Connection,
    scope: &Value,
    job: &str,
    original_count: usize,
) -> Result<bool> {
    Ok(original_count == 1
        && scope.get("hadPreparing").and_then(Value::as_bool) == Some(false)
        && scope
            .get("ingresses")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        && crate::prompt_intake::get_in(db, job)?.is_none())
}

fn same_original(before: &StoredQueueJob, after: &StoredQueueJob) -> bool {
    before.state == QueueJobState::Starting
        && before.turn_id.is_none()
        && !before.goal_waiting
        && before.attempt_count > 0
        && before.app_server_generation > 0
        && before.execution_generation == Some(before.app_server_generation)
        && after.state == QueueJobState::Running
        && !after.goal_waiting
        && after.turn_observation_generation == Some(before.app_server_generation)
        && before.job_id == after.job_id
        && before.target_thread_id == after.target_thread_id
        && before.channel_id == after.channel_id
        && before.owner_user_id == after.owner_user_id
        && before.discord_message_id == after.discord_message_id
        && before.prompt == after.prompt
        && before.created_at.to_bits() == after.created_at.to_bits()
        && before.last_error == after.last_error
        && before.attempt_count == after.attempt_count
        && before.app_server_generation == after.app_server_generation
        && before.execution_generation == after.execution_generation
        && before.baseline_turn_ids == after.baseline_turn_ids
}

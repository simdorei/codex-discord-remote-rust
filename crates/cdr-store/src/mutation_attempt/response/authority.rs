use super::{Authority, Scope, digest, refused, unheld_except};
use crate::{Result, queue};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

fn identity(scope: &Scope<'_>) -> Result<(String, String, String)> {
    let params = &scope.request["params"];
    let thread = params["threadId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.trim() == *s)
        .ok_or_else(refused)?;
    let turn = params["turnId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.trim() == *s)
        .ok_or_else(refused)?;
    if scope.runtime.is_empty()
        || scope.resident.is_empty()
        || scope.generation < 1
        || scope.request["id"].is_null()
        || scope.request["occurrence"].is_null()
        || scope.request["method"].as_str().is_none_or(str::is_empty)
    {
        return Err(refused());
    }
    let key = digest(&json!([
        scope.runtime,
        scope.resident,
        scope.generation,
        scope.request["id"],
        scope.request["occurrence"]
    ]))?;
    Ok((key, thread.into(), turn.into()))
}

pub fn capture(path: &Path, scope: &Scope<'_>) -> Result<Value> {
    let (key, thread, turn) = identity(scope)?;
    let mut db = super::super::existing(path)?;
    let tx = db.transaction()?;
    let ids = tx
        .prepare(
            "SELECT job_id FROM codex_turn_queue
        WHERE target_thread_id=? AND turn_id=? LIMIT 2",
        )?
        .query_map(params![thread, turn], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ids.len() != 1 {
        return Err(refused());
    }
    let job = queue::select_job(&tx, &ids[0])?;
    let authority = Authority {
        key,
        runtime: scope.runtime.into(),
        resident: scope.resident.into(),
        generation: scope.generation,
        request_sha256: digest(scope.request)?,
        thread,
        turn,
        job: job.job_id.clone(),
        original_job: serde_json::to_string(&job)?,
        mapping: crate::mapping::mirrored_thread_id_in(&tx, Some(job.channel_id))?,
        stop_sequence: stop_sequence(&tx, &job.target_thread_id)?,
    };
    validate(&tx, scope, &authority, "")?;
    tx.commit()?;
    Ok(serde_json::to_value(authority)?)
}

pub(super) fn check_identity(scope: &Scope<'_>, authority: &Authority) -> Result<()> {
    let (key, thread, turn) = identity(scope)?;
    if authority.key != key
        || authority.runtime != scope.runtime
        || authority.resident != scope.resident
        || authority.generation != scope.generation
        || authority.thread != thread
        || authority.turn != turn
        || authority.request_sha256 != digest(scope.request)?
    {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn validate(
    db: &Connection,
    scope: &Scope<'_>,
    authority: &Authority,
    own: &str,
) -> Result<()> {
    check_identity(scope, authority)?;
    super::super::owner_is_current(db, scope.runtime)?;
    super::super::unblocked(db, Some(&authority.thread))?;
    unheld_except(db, &authority.thread, own)?;
    crate::ingress::stop::control::require_unheld_in(db, &authority.thread)?;
    crate::execution_hold::require_unheld_in(db, &authority.job)?;
    let job = queue::select_job(db, &authority.job)?;
    if serde_json::to_string(&job)? != authority.original_job
        || job.state != queue::QueueJobState::Running
        || job.goal_waiting
        || job.app_server_generation != scope.generation
        || job.channel_id <= 0
        || job.owner_user_id.is_none_or(|id| id <= 0)
        || crate::mapping::mirrored_thread_id_in(db, Some(job.channel_id))? != authority.mapping
        || authority
            .mapping
            .as_ref()
            .is_some_and(|mapped| mapped != &authority.thread)
        || stop_sequence(db, &authority.thread)? != authority.stop_sequence
    {
        return Err(refused());
    }
    let valid: bool = db.query_row("SELECT
        (SELECT count(*) FROM codex_turn_queue WHERE target_thread_id=?1 AND turn_id=?2)=1
        AND NOT EXISTS(SELECT 1 FROM codex_request_cancellations WHERE job_id=?3
            OR (?4 IS NOT NULL AND discord_message_id=?4))
        AND NOT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)
        AND NOT EXISTS(SELECT 1 FROM codex_dead_generation_holds WHERE target_thread_id=?1)
        AND NOT EXISTS(SELECT 1 FROM codex_dead_generation_incidents WHERE runtime_id=?5 AND generation=?6)
        AND NOT EXISTS(SELECT 1 FROM codex_observed_completions WHERE thread_id=?1 AND turn_id=?2)",
        params![authority.thread,authority.turn,authority.job,job.discord_message_id,
            scope.runtime,scope.generation], |row| row.get(0))?;
    if !valid {
        return Err(refused());
    }
    Ok(())
}

fn stop_sequence(db: &Connection, thread: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM cdr_stop_controls
        WHERE target_thread_id=?",
        [thread],
        |row| row.get(0),
    )?)
}

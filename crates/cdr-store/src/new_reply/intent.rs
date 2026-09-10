use super::{Identity, NewReply, get_in};
use crate::{
    Result, StoreError,
    ingress::StoredIngress,
    queue::{NewQueueJob, StoredQueueJob},
    schema::open_initialized,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

/// Freeze the exact acknowledgement before prompt preparation or turn/start.
pub(crate) fn seed_in(
    connection: &Connection,
    ingress: &str,
    job: &str,
    state_db: &Path,
    acknowledgement: &str,
) -> Result<()> {
    let seed = json!({"state_db":state_db.to_string_lossy(),"acknowledgement":acknowledgement});
    let changed = connection.execute(
        "UPDATE discord_ingress_journal SET outcome_json=json_set(outcome_json,'$.new_reply_seed',json(?))
         WHERE ingress_id=? AND owner_kind='prompt' AND owner_id=?
         AND json_extract(outcome_json,'$.new_creation.version')=1
         AND json_type(outcome_json,'$.new_reply_seed') IS NULL",
        params![seed.to_string(),ingress,job],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "new acknowledgement seed has no unique original owner".into(),
        ));
    }
    Ok(())
}

pub(crate) fn promote_in(
    connection: &Connection,
    saved: &StoredIngress,
    job: &NewQueueJob<'_>,
    digest: &str,
) -> Result<()> {
    let Some(seed) = saved.outcome.as_ref().and_then(|v| v.get("new_reply_seed")) else {
        // Historical rows are not eligible for automatic backfill/replay.
        return Ok(());
    };
    let creation = saved
        .outcome
        .as_ref()
        .and_then(|v| v.get("new_creation"))
        .ok_or_else(|| StoreError::Integrity("new creation evidence is missing".into()))?;
    let value = |v: &Value, key: &str| -> Result<String> {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| StoreError::Integrity(format!("new evidence field missing: {key}")))
    };
    let identity = Identity {
        ingress_id: saved.ingress_id.clone(),
        job_id: job.job_id.into(),
        thread_id: job.target_thread_id.into(),
        cwd: value(creation, "cwd")?,
        state_db: value(seed, "state_db")?,
        channel_id: job.channel_id,
        origin_channel_id: saved.channel_id,
        event_id: saved.event_id.or(saved.source_message_id),
        kind: saved.kind,
        creation_generation: saved
            .outcome
            .as_ref()
            .and_then(|v| v.get("thread_start_generation"))
            .and_then(Value::as_i64)
            .ok_or_else(|| StoreError::Integrity("new creation generation missing".into()))?,
        prompt_sha256: digest.into(),
        acknowledgement: value(seed, "acknowledgement")?,
    };
    if creation.get("version").and_then(Value::as_u64) != Some(1)
        || creation.get("origin_channel_id").and_then(Value::as_i64) != Some(saved.channel_id)
    {
        return Err(StoreError::Integrity(
            "new creation identity changed".into(),
        ));
    }
    if let Some(old) = get_in(connection, job.job_id)? {
        if old.identity != identity {
            return Err(StoreError::Integrity(
                "immutable new first reply identity changed".into(),
            ));
        }
        return Ok(());
    }
    connection.execute(
        "INSERT INTO codex_new_first_replies(job_id,ingress_id,identity_json) VALUES(?,?,?)",
        params![
            job.job_id,
            saved.ingress_id,
            serde_json::to_string(&identity)?
        ],
    )?;
    Ok(())
}

/// Called under the same writer transaction as the generation-bound Running CAS.
pub(crate) fn bind_running_in(connection: &Connection, job: &StoredQueueJob) -> Result<()> {
    if job.state != crate::queue::QueueJobState::Running {
        return Ok(());
    }
    let Some(record) = get_in(connection, &job.job_id)? else {
        return Ok(());
    };
    let Some(turn) = job.turn_id.as_deref() else {
        return Ok(());
    };
    if record.identity.thread_id != job.target_thread_id
        || record.identity.channel_id != job.channel_id
    {
        return Err(StoreError::Integrity(
            "new first-turn binding changed its destination".into(),
        ));
    }
    if record.turn_id.is_some() {
        return Ok(());
    } // Goal continuation never replaces the first turn.
    connection.execute("UPDATE codex_new_first_replies SET turn_id=?,accepted_at=?,version=version+1 WHERE job_id=? AND turn_id IS NULL",
        params![turn,job.updated_at,job.job_id])?;
    Ok(())
}

pub fn get(path: &Path, job: &str) -> Result<Option<NewReply>> {
    get_in(&open_initialized(path)?, job)
}

pub fn get_by_ingress(path: &Path, ingress: &str) -> Result<Option<NewReply>> {
    let connection = open_initialized(path)?;
    let job: Option<String> = connection
        .query_row(
            "SELECT job_id FROM codex_new_first_replies WHERE ingress_id=?",
            [ingress],
            |row| row.get(0),
        )
        .optional()?;
    job.map(|job| get_in(&connection, &job))
        .transpose()
        .map(Option::flatten)
}

use crate::{Result, StoreError};
use rusqlite::{Connection, params};

pub(super) fn cancel_unstarted_ingress(
    db: &Connection,
    job: &str,
    event: Option<i64>,
    target: &str,
    channel: i64,
    owner: i64,
    now: f64,
) -> Result<()> {
    let key = job.strip_prefix("ingress:").ok_or_else(|| {
        StoreError::Integrity("invalid pending ingress cancellation identity".into())
    })?;
    let occurrence: bool = db.query_row(
        "SELECT
        EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?1 OR discord_message_id=?2) OR
        EXISTS(SELECT 1 FROM codex_prompt_intakes WHERE job_id=?1 OR discord_message_id=?2)",
        params![job, event],
        |row| row.get(0),
    )?;
    if occurrence {
        return Err(StoreError::Integrity(
            "ingress has conflicting execution ownership; nothing was cancelled".into(),
        ));
    }
    db.execute("INSERT INTO codex_request_cancellations
        (job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at) VALUES (?,?,?,?,?,?)",
        params![job,target,channel,owner,event,now])?;
    let changed = db.execute(
        "UPDATE discord_ingress_journal SET state='completed',phase='cancelled',
        owner_kind='cancellation',owner_id=?,outcome_json=json_set(
          CASE WHEN json_type(outcome_json)='object' THEN outcome_json
          WHEN outcome_json IS NULL OR json_type(outcome_json)='null' THEN '{}'
          ELSE json_object('prior_result',json(outcome_json)) END,
          '$.kind','request_cancelled','$.job_id',?),updated_at=? WHERE ingress_id=?
        AND event_id IS ? AND target_thread_id=? AND channel_id=? AND owner_user_id=?
        AND state IN ('staged','acknowledged') AND owner_id IS NULL",
        params![job, job, now, key, event, target, channel, owner],
    )?;
    if changed != 1 {
        return Err(StoreError::Integrity(
            "ingress execution already started or cancellation ownership changed".into(),
        ));
    }
    Ok(())
}

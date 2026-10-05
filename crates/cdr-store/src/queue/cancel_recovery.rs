//! Explicit recovery cancellation also covers started work. Evidence is retained;
//! the external recovery controller must prove process exit separately.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{TransactionBehavior, params};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct RecoveryCancellation {
    pub jobs: Vec<String>,
    pub started_or_uncertain: usize,
}

pub fn cancel_for_recovery(
    path: &Path,
    target: &str,
    channel: i64,
    owner: i64,
    now: f64,
) -> Result<RecoveryCancellation> {
    cancel_for_recovery_checked(path, target, channel, owner, now, &|_| Ok(()))
}

pub fn cancel_for_recovery_checked(
    path: &Path,
    target: &str,
    channel: i64,
    owner: i64,
    now: f64,
    check: &(dyn Fn(&rusqlite::Connection) -> Result<()> + Sync),
) -> Result<RecoveryCancellation> {
    if target.is_empty() || channel <= 0 || owner <= 0 || !now.is_finite() || now < 0.0 {
        return Err(StoreError::Integrity(
            "invalid recovery cancellation scope".into(),
        ));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check(&tx)?;
    super::fork_handoff::ensure_no_unresolved_handoff(&tx, target)?;
    super::fork_handoff::ensure_source_not_moved(&tx, target)?;
    let rows = tx
        .prepare(
            "SELECT job_id,'queue',channel_id,owner_user_id,discord_message_id
        FROM codex_turn_queue WHERE target_thread_id=?1 UNION ALL
        SELECT job_id,'intake',channel_id,owner_user_id,discord_message_id
        FROM codex_prompt_intakes WHERE target_thread_id=?1",
        )?
        .query_map([target], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > 128 || rows.iter().any(|r| r.2 != channel || r.3 != Some(owner)) {
        return Err(StoreError::Integrity(
            "recovery request scope contains another sender/channel or too many requests".into(),
        ));
    }
    let mut result = RecoveryCancellation {
        jobs: Vec::new(),
        started_or_uncertain: 0,
    };
    let scope = crate::ingress::stop::StopScope {
        target,
        channel,
        owner,
    };
    for (job, kind, _, _, event) in rows {
        cancel_request_in(&tx, scope, now, job, &kind, event, &mut result)?;
    }
    // A preparer which has not produced a queue row must also lose replay authority.
    let unowned = tx.prepare("SELECT ingress_id,event_id,channel_id,owner_user_id,payload_json,state
        FROM discord_ingress_journal WHERE target_thread_id=? AND owner_id IS NULL
        AND state IN ('staged','acknowledged','executing','held')
        AND json_type(payload_json,'$.version')='integer' AND json_extract(payload_json,'$.version')=1 AND (
            (kind='message' AND (json_type(payload_json,'$.plan.Execute.Ask.prompt')='text'
                OR json_type(payload_json,'$.plan.Execute.Interview.prompt')='text'))
            OR (kind='interaction' AND json_extract(payload_json,'$.work.Slash.name') IN ('ask','interview')
                AND json_type(payload_json,'$.work.Slash.values.prompt.String')='text'))")?
        .query_map([target], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,
            r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if result.jobs.len() + unowned.len() > 128
        || unowned.iter().any(|r| r.2 != channel || r.3 != owner)
    {
        return Err(StoreError::Integrity(
            "recovery ingress scope changed; cancellation rolled back".into(),
        ));
    }
    for (key, event, _, _, payload, state) in unowned {
        let job = format!("ingress:{key}");
        crate::execution_hold::hold_in(&tx,&job,target,"user requested full recovery; do not replay",
            &json!({"ingress_id":key,"payload":serde_json::from_str::<serde_json::Value>(&payload)?,"state":state}).to_string())?;
        tx.execute(
            "INSERT INTO codex_request_cancellations
            (job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at)
            VALUES (?,?,?,?,?,?)",
            params![job, target, channel, owner, event, now],
        )?;
        tx.execute("UPDATE discord_ingress_journal SET state='completed',phase='cancelled',
            owner_kind='cancellation',owner_id=?,outcome_json=json_set(CASE WHEN json_type(outcome_json)='object'
            THEN outcome_json ELSE '{}' END,'$.kind','request_cancelled','$.job_id',?),updated_at=? WHERE ingress_id=?",
            params![job,job,now,key])?;
        if state == "executing" || state == "held" {
            result.started_or_uncertain += 1;
        }
        result.jobs.push(job);
    }
    check(&tx)?;
    crate::ingress::stop::revision::record_recovery_in(&tx, scope, &result.jobs, now)?;
    tx.commit()?;
    Ok(result)
}

fn cancel_request_in(
    tx: &rusqlite::Connection,
    scope: crate::ingress::stop::StopScope<'_>,
    now: f64,
    job: String,
    kind: &str,
    event: Option<i64>,
    result: &mut RecoveryCancellation,
) -> Result<()> {
    let crate::ingress::stop::StopScope {
        target,
        channel,
        owner,
    } = scope;
    let count: i64 = tx.query_row("SELECT
        (SELECT count(*) FROM codex_turn_queue WHERE job_id=?1 OR (?2 IS NOT NULL AND discord_message_id=?2)) +
        (SELECT count(*) FROM codex_prompt_intakes WHERE job_id=?1 OR (?2 IS NOT NULL AND discord_message_id=?2))",
        params![job,event], |r| r.get(0))?;
    if count != 1 {
        return Err(StoreError::Integrity(
            "conflicting recovery request identity".into(),
        ));
    }
    let owners = crate::ingress::cancellation_owners(tx, &job, event, target, channel, owner)?;
    let evidence = if kind == "queue" {
        let stored = super::read::select_job(tx, &job)?;
        if stored.state != super::QueueJobState::Pending
            || stored.execution_generation.is_some()
            || stored.turn_id.is_some()
            || stored.attempt_count > 0
        {
            result.started_or_uncertain += 1;
        }
        serde_json::to_value(stored)?
    } else {
        let saved: String = tx.query_row(
            "SELECT json_object(
            'job_id',job_id,'target_thread_id',target_thread_id,'channel_id',channel_id,
            'owner_user_id',owner_user_id,'discord_message_id',discord_message_id,
            'raw_prompt',raw_prompt,'auto_queue_when_busy',auto_queue_when_busy,
            'require_current_mirror',require_current_mirror,'attempt_count',attempt_count,
            'last_error',last_error,'retry_after',retry_after,'claim_token',claim_token,
            'claim_expires_at',claim_expires_at,'created_at',created_at,'updated_at',updated_at)
            FROM codex_prompt_intakes WHERE job_id=?",
            [&job],
            |r| r.get(0),
        )?;
        serde_json::from_str(&saved)?
    };
    crate::execution_hold::hold_in(
        tx,
        &job,
        target,
        "user requested full recovery; cancelled, never replay; prior effects are not rolled back",
        &json!({"kind":kind,"request":evidence,"cancelled_at":now}).to_string(),
    )?;
    tx.execute(
        "INSERT INTO codex_request_cancellations
        (job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at)
        VALUES (?,?,?,?,?,?)",
        params![job, target, channel, owner, event, now],
    )?;
    let table = if kind == "queue" {
        "codex_turn_queue"
    } else {
        "codex_prompt_intakes"
    };
    tx.execute(&format!("DELETE FROM {table} WHERE job_id=?"), [&job])?;
    for key in owners {
        tx.execute("UPDATE discord_ingress_journal SET state='completed',phase='cancelled',
            outcome_json=json_set(CASE WHEN json_type(outcome_json)='object' THEN outcome_json
            ELSE '{}' END,'$.kind','request_cancelled','$.job_id',?),updated_at=? WHERE ingress_id=?",
            params![job,now,key])?;
    }
    result.jobs.push(job);
    Ok(())
}

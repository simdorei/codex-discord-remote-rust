//! One transactional cancellation boundary shared with intake promotion/queue claims.
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;

struct Candidate {
    job: String,
    kind: String,
    event: Option<i64>,
    eligible: bool,
}

pub fn cancel_latest_pending(
    path: &Path,
    target: &str,
    channel: i64,
    owner: i64,
    now: f64,
) -> Result<Option<String>> {
    cancel_latest_pending_on_route(path, target, channel, owner, now, false)
}

pub fn cancel_latest_pending_on_route(
    path: &Path,
    target: &str,
    channel: i64,
    owner: i64,
    now: f64,
    require_mirror: bool,
) -> Result<Option<String>> {
    if !now.is_finite() || now < 0.0 {
        return Err(StoreError::Integrity(
            "invalid cancellation timestamp".into(),
        ));
    }
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if require_mirror
        && crate::mapping::mirrored_thread_id_in(&tx, Some(channel))?.as_deref() != Some(target)
    {
        return Err(StoreError::Integrity(
            "retract room mapping changed; nothing was cancelled".into(),
        ));
    }
    super::fork_handoff::ensure_no_unresolved_handoff(&tx, target)?;
    super::fork_handoff::ensure_source_not_moved(&tx, target)?;
    crate::dead_generation::ensure_target_available(&tx, target)?;
    let candidates = candidates(&tx, target, channel, owner)?;
    let Some(selected) = candidates.iter().find(|item| item.eligible) else {
        if !candidates.is_empty() {
            return Err(StoreError::Integrity(
                "request execution has started or its outcome is unknown; nothing was cancelled"
                    .into(),
            ));
        }
        tx.commit()?;
        return Ok(None);
    };
    if selected.kind == "ingress" {
        super::cancel_ingress::cancel_unstarted_ingress(
            &tx,
            &selected.job,
            selected.event,
            target,
            channel,
            owner,
            now,
        )?;
        let id = selected.job.clone();
        tx.commit()?;
        return Ok(Some(id));
    }
    let owners = ensure_unambiguous(&tx, selected, target, channel, owner)?;
    tx.execute(
        "INSERT INTO codex_request_cancellations
        (job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at)
        VALUES (?,?,?,?,?,?)",
        params![selected.job, target, channel, owner, selected.event, now],
    )?;
    let table = if selected.kind == "intake" {
        "codex_prompt_intakes"
    } else {
        "codex_turn_queue"
    };
    if tx.execute(
        &format!("DELETE FROM {table} WHERE job_id=?"),
        [&selected.job],
    )? != 1
    {
        return Err(StoreError::Integrity(
            "cancellation ownership changed".into(),
        ));
    }
    for key in owners {
        tx.execute(
            "UPDATE discord_ingress_journal SET state='completed',phase='cancelled',
        outcome_json=json_set(CASE WHEN json_type(outcome_json)='object' THEN outcome_json
          WHEN outcome_json IS NULL OR json_type(outcome_json)='null' THEN '{}'
          ELSE json_object('prior_result',json(outcome_json)) END,
          '$.kind','request_cancelled','$.job_id',?),updated_at=? WHERE ingress_id=?",
            params![selected.job, now, key],
        )?;
    }
    let id = selected.job.clone();
    tx.commit()?;
    Ok(Some(id))
}

fn candidates(db: &Connection, target: &str, channel: i64, owner: i64) -> Result<Vec<Candidate>> {
    let mut query = db.prepare(
        "SELECT job_id,kind,discord_message_id,eligible FROM (
        SELECT job_id,'queue' AS kind,discord_message_id,created_at,
            state='pending' AND attempt_count=0 AND turn_id IS NULL AS eligible
        FROM codex_turn_queue WHERE target_thread_id=?1 AND channel_id=?2 AND owner_user_id=?3
        UNION ALL
        SELECT job_id,'intake',discord_message_id,created_at,1
        FROM codex_prompt_intakes WHERE target_thread_id=?1 AND channel_id=?2 AND owner_user_id=?3
        UNION ALL
        SELECT 'ingress:' || ingress_id,'ingress',event_id,created_at,state IN ('staged','acknowledged')
        FROM discord_ingress_journal WHERE target_thread_id=?1 AND channel_id=?2 AND owner_user_id=?3
        AND owner_id IS NULL AND state IN ('staged','acknowledged','executing','held')
        AND json_type(payload_json,'$.version')='integer' AND json_extract(payload_json,'$.version')=1 AND (
          (kind='message' AND (json_type(payload_json,'$.plan.Execute.Ask.prompt')='text'
                           OR json_type(payload_json,'$.plan.Execute.Interview.prompt')='text'))
          OR (kind='interaction' AND json_extract(payload_json,'$.work.Slash.name') IN ('ask','interview')
              AND json_type(payload_json,'$.work.Slash.values.prompt.String')='text')))
        ORDER BY created_at DESC,job_id DESC,kind",
    )?;
    Ok(query
        .query_map(params![target, channel, owner], |row| {
            Ok(Candidate {
                job: row.get(0)?,
                kind: row.get(1)?,
                event: row.get(2)?,
                eligible: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn ensure_unambiguous(
    db: &Connection,
    item: &Candidate,
    target: &str,
    channel: i64,
    owner: i64,
) -> Result<Vec<String>> {
    let occurrences: i64 = db.query_row("SELECT
        (SELECT COUNT(*) FROM codex_turn_queue WHERE job_id=?1 OR (?2 IS NOT NULL AND discord_message_id=?2)) +
        (SELECT COUNT(*) FROM codex_prompt_intakes WHERE job_id=?1 OR (?2 IS NOT NULL AND discord_message_id=?2))",
        params![item.job,item.event], |row| row.get(0))?;
    let uncertain: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_delivery_outbox WHERE job_id=?)",
        params![item.job],
        |row| row.get(0),
    )?;
    if occurrences != 1 || uncertain {
        return Err(StoreError::Integrity(
            "request has conflicting or uncertain ownership evidence; nothing was cancelled".into(),
        ));
    }
    crate::ingress::cancellation_owners(db, &item.job, item.event, target, channel, owner)
}

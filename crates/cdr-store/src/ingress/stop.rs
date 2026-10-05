//! Bounded stop acceptance for original queued requests, not proof of process exit.
pub mod control;
mod intake;
pub mod revision;
pub(crate) mod unowned;
use super::{IngressKind, StoredIngress, read::get_in};
use crate::{Result, StoreError, queue::QueueJobState};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
pub struct StopScope<'a> {
    pub target: &'a str,
    pub channel: i64,
    pub owner: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct StopReceipt {
    pub jobs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ingresses: Vec<String>,
}

const REASON: &str =
    "user requested stop; original request held, never replay; execution end unconfirmed";

fn refused() -> StoreError {
    StoreError::Integrity(
        "stop custody differs or could not be preserved; no stop acceptance".into(),
    )
}

/// No target lock, RPC, migration or queue rewrite occurs in this transaction.
pub fn accept_nonrunning(
    path: &Path,
    scope: StopScope<'_>,
    binding: &Value,
    expected: Option<&StoredIngress>,
    check_selected: impl Fn() -> Result<()>,
) -> Result<Option<StopReceipt>> {
    accept_scope(path, scope, binding, expected, check_selected, false)
}

/// Save bounded intent without inferring an active turn or requiring a resident.
/// Empty local scope is not proof of idle. Never rewrite or replay an old job.
pub fn accept_unresolved(
    path: &Path,
    scope: StopScope<'_>,
    binding: &Value,
    expected: Option<&StoredIngress>,
    check_selected: impl Fn() -> Result<()>,
) -> Result<Option<StopReceipt>> {
    accept_scope(path, scope, binding, expected, check_selected, true)
}

fn accept_scope(
    path: &Path,
    scope: StopScope<'_>,
    binding: &Value,
    expected: Option<&StoredIngress>,
    check_selected: impl Fn() -> Result<()>,
    allow_uncertain: bool,
) -> Result<Option<StopReceipt>> {
    if scope.target.trim().is_empty()
        || scope.channel <= 0
        || scope.owner <= 0
        || expected.is_some_and(|record| record.phase != "processing")
    {
        return Err(refused());
    }
    match std::fs::metadata(path) {
        // Legacy direct controls may have no queue database yet. This is not a receipt.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && expected.is_none() => {
            return Ok(None);
        }
        Err(error) => {
            return Err(StoreError::Integrity(format!(
                "stop database unavailable: {error}"
            )));
        }
        Ok(_) => {}
    }
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(Duration::from_millis(500))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate(&tx, scope, binding, expected)?;
    check_selected()?;
    let ids = target_jobs(&tx, scope.target)?;
    let preparing = intake::snapshot(&tx, scope, ids.len())?;
    let unowned = unowned::snapshot(&tx, scope, ids.len() + preparing.len())?;
    if ids.len() > 128 {
        return Err(refused());
    }
    let jobs = ids
        .iter()
        .map(|id| crate::queue::select_job(&tx, id))
        .collect::<Result<Vec<_>>>()?;
    if jobs
        .iter()
        .any(|job| job.channel_id != scope.channel || job.owner_user_id != Some(scope.owner))
    {
        return Err(refused());
    }
    if !allow_uncertain
        && ((jobs.is_empty() && preparing.is_empty() && unowned.is_empty())
            || jobs
                .iter()
                .any(|job| !matches!(job.state, QueueJobState::Pending | QueueJobState::Starting)))
    {
        return Ok(None);
    }
    let mut holds = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let original = hold_snapshot(&tx, &job.job_id)?;
        let wanted = original.unwrap_or_else(|| {
            (
                scope.target.to_owned(),
                REASON.to_owned(),
                json!({"kind":"stop","request":job,"ingress_id":expected.map(|r| &r.ingress_id)})
                    .to_string(),
            )
        });
        if wanted.0 != scope.target {
            return Err(refused());
        }
        crate::execution_hold::hold_in(&tx, &job.job_id, scope.target, &wanted.1, &wanted.2)?;
        holds.push(wanted);
    }
    validate(&tx, scope, binding, expected)?;
    let intake_holds = intake::hold(
        &tx,
        scope,
        &preparing,
        expected.map(|r| r.ingress_id.as_str()),
    )?;
    let operation = expected.map_or_else(
        || format!("stop:{}", uuid::Uuid::new_v4()),
        |record| format!("stop:{}", record.ingress_id),
    );
    let held = unowned::hold(&tx, &unowned, &operation)?;
    let receipt = StopReceipt {
        jobs: intake::receipt_ids(&ids, &preparing),
        ingresses: held
            .iter()
            .map(|record| record.ingress_id.clone())
            .collect(),
    };
    let claimed = claim_record(&tx, expected, &receipt)?;
    let revision = revision::advance_in(&tx, scope, binding, &receipt, &operation)?;
    validate(&tx, scope, binding, claimed.as_ref())?;
    check_selected()?;
    if target_jobs(&tx, scope.target)? != ids {
        return Err(refused());
    }
    verify_job_holds(&tx, &jobs, &holds)?;
    intake::verify(&tx, scope, &preparing, &intake_holds)?;
    unowned::verify(&tx, scope.target, &held)?;
    revision::verify_in(&tx, &revision)?;
    tx.commit()?;
    Ok(Some(receipt))
}

fn verify_job_holds(
    db: &Connection,
    jobs: &[crate::queue::StoredQueueJob],
    holds: &[(String, String, String)],
) -> Result<()> {
    for (job, hold) in jobs.iter().zip(holds) {
        if crate::queue::select_job(db, &job.job_id)? != *job
            || hold_snapshot(db, &job.job_id)?.as_ref() != Some(hold)
        {
            return Err(refused());
        }
    }
    Ok(())
}

fn target_jobs(db: &Connection, target: &str) -> Result<Vec<String>> {
    Ok(db.prepare(
        "SELECT job_id FROM codex_turn_queue WHERE target_thread_id=? ORDER BY job_id LIMIT 129",
    )?.query_map([target], |row| row.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn hold_snapshot(db: &Connection, job: &str) -> Result<Option<(String, String, String)>> {
    Ok(db
        .query_row(
            "SELECT target_thread_id,reason,evidence_json FROM cdr_execution_holds WHERE job_id=?",
            [job],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

fn validate(
    db: &Connection,
    scope: StopScope<'_>,
    binding: &Value,
    expected: Option<&StoredIngress>,
) -> Result<()> {
    let command = binding
        .get("command")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let fields = command
        .get("Stop")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    let reference = fields.get("reference").ok_or_else(refused)?;
    let explicit = reference
        .as_str()
        .is_some_and(|value| !value.trim().is_empty());
    if command.len() != 1
        || fields.len() != 1
        || (!explicit && !reference.is_null())
        || binding.get("target").and_then(Value::as_str) != Some(scope.target)
    {
        return Err(refused());
    }
    let route_valid = match binding.get("route").and_then(Value::as_str) {
        Some("Explicit") => explicit,
        Some("Mapped") if !explicit => {
            crate::mapping::mirrored_thread_id_in(db, Some(scope.channel))?.as_deref()
                == Some(scope.target)
        }
        Some("Selected") if !explicit => {
            crate::mapping::mirrored_thread_id_in(db, Some(scope.channel))?.is_none()
        }
        _ => false,
    };
    if !route_valid {
        return Err(refused());
    }
    if let Some(record) = expected {
        let event = record
            .event_id
            .filter(|value| *value > 0)
            .ok_or_else(refused)?;
        if get_in(db, &record.ingress_id)?.as_ref() != Some(record)
            || record.kind != IngressKind::Message
            || record.ingress_id != format!("message:{event}")
            || record.source_message_id != Some(event)
            || record.channel_id != scope.channel
            || record.owner_user_id != scope.owner
            || record.state != "executing"
            || record.owner_id.is_some()
            || record.owner_kind.is_some()
            || record.target_thread_id.as_deref() != Some(scope.target)
            || record.payload.get("version") != Some(&json!(1))
            || record.payload.get("lifecycle_binding") != Some(binding)
            || record.payload.pointer("/plan/Execute") != binding.get("command")
        {
            return Err(refused());
        }
    }
    Ok(())
}

fn claim_record(
    db: &Connection,
    expected: Option<&StoredIngress>,
    receipt: &StopReceipt,
) -> Result<Option<StoredIngress>> {
    let Some(expected) = expected else {
        return Ok(None);
    };
    let mut claimed = expected.clone();
    claimed.phase = "stop_accepted".into();
    claimed.updated_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    claimed.outcome = Some(json!({
        "kind":"stop_accepted","jobs":receipt.jobs,"ingresses":receipt.ingresses,"execution_end_confirmed":false
    }));
    if db.execute(
        "UPDATE discord_ingress_journal SET phase=?,outcome_json=?,updated_at=?
         WHERE ingress_id=? AND state='executing' AND phase='processing' AND owner_id IS NULL",
        params![
            claimed.phase,
            claimed.outcome.as_ref().map(Value::to_string),
            claimed.updated_at,
            claimed.ingress_id
        ],
    )? != 1
    {
        return Err(refused());
    }
    Ok(Some(claimed))
}

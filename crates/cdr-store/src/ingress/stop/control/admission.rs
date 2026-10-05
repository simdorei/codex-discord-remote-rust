use super::super::{
    StopReceipt, StopScope, StoredIngress, claim_record, hold_snapshot, target_jobs, validate,
};
use super::{StopControl, existing, refused, retained_in};
use crate::{
    Result,
    queue::{QueueJobState, StoredQueueJob},
};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::Path;

/// Accept without an async lock, server lookup, migration, or queue rewrite.
pub fn accept_running(
    path: &Path,
    scope: StopScope<'_>,
    binding: &Value,
    expected: Option<&StoredIngress>,
    resident: (&str, i64),
    check_selected: impl Fn() -> Result<()>,
) -> Result<Option<StopControl>> {
    if scope.target.trim().is_empty()
        || scope.channel <= 0
        || scope.owner <= 0
        || resident.0.is_empty()
        || resident.1 <= 0
        || expected.is_some_and(|record| record.phase != "processing")
    {
        return Err(refused());
    }
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && expected.is_none() => {
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let mut db = existing(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate(&tx, scope, binding, expected)?;
    check_selected()?;
    let ids = target_jobs(&tx, scope.target)?;
    if ids.len() > 128 {
        return Err(refused());
    }
    let jobs = ids
        .iter()
        .map(|id| crate::queue::select_job(&tx, id))
        .collect::<Result<Vec<_>>>()?;
    if !jobs.iter().any(|job| job.state == QueueJobState::Running) {
        return Ok(None);
    }
    let running = running_job(&tx, scope, &jobs)?;
    let preparing = super::super::intake::snapshot(&tx, scope, ids.len())?;
    let unowned = super::super::unowned::snapshot(&tx, scope, ids.len() + preparing.len())?;
    let control = StopControl {
        operation_id: expected.map_or_else(
            || format!("stop:{}", uuid::Uuid::new_v4()),
            |record| format!("stop:{}", record.ingress_id),
        ),
        target: scope.target.into(),
        channel: scope.channel,
        owner: scope.owner,
        resident: resident.0.into(),
        generation: resident.1,
        turn: running.turn_id.clone().ok_or_else(refused)?,
        binding: binding.clone(),
        jobs: jobs
            .iter()
            .map(serde_json::to_string)
            .collect::<serde_json::Result<_>>()?,
        can_settle: jobs.iter().all(|job| {
            definitely_unstarted(job)
                || (job.job_id == running.job_id
                    && job.completion_evidence_generation() == resident.1
                    && !job.goal_waiting)
        }),
    };
    let holds = hold_originals(&tx, scope, &jobs, &control.operation_id)?;
    let intake_holds =
        super::super::intake::hold(&tx, scope, &preparing, Some(&control.operation_id))?;
    tx.execute(
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
    )?;
    validate(&tx, scope, binding, expected)?;
    let held = super::super::unowned::hold(&tx, &unowned, &control.operation_id)?;
    let receipt = StopReceipt {
        jobs: super::super::intake::receipt_ids(&ids, &preparing),
        ingresses: held
            .iter()
            .map(|record| record.ingress_id.clone())
            .collect(),
    };
    let claimed = claim_record(&tx, expected, &receipt)?;
    let revision =
        super::super::revision::advance_in(&tx, scope, binding, &receipt, &control.operation_id)?;
    validate(&tx, scope, binding, claimed.as_ref())?;
    check_selected()?;
    if target_jobs(&tx, scope.target)? != ids || !retained_in(&tx, &control)? {
        return Err(refused());
    }
    super::super::verify_job_holds(&tx, &jobs, &holds)?;
    super::super::intake::verify(&tx, scope, &preparing, &intake_holds)?;
    super::super::unowned::verify(&tx, scope.target, &held)?;
    super::super::revision::verify_in(&tx, &revision)?;
    tx.commit()?;
    Ok(Some(control))
}

fn running_job<'a>(
    db: &Connection,
    scope: StopScope<'_>,
    jobs: &'a [StoredQueueJob],
) -> Result<&'a StoredQueueJob> {
    if jobs.iter().any(|job| {
        job.channel_id != scope.channel
            || job.owner_user_id != Some(scope.owner)
            || job.state == QueueJobState::Quarantined
    }) {
        return Err(refused());
    }
    let running: Vec<_> = jobs
        .iter()
        .filter(|job| job.state == QueueJobState::Running)
        .collect();
    if running.len() != 1 {
        return Err(refused());
    }
    let job = running[0];
    let turn = job
        .turn_id
        .as_deref()
        .filter(|turn| !turn.is_empty())
        .ok_or_else(refused)?;
    let terminal: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_observed_completions
        WHERE thread_id=? AND turn_id=?)",
        params![scope.target, turn],
        |row| row.get(0),
    )?;
    if terminal {
        return Err(refused());
    }
    Ok(job)
}

fn definitely_unstarted(job: &StoredQueueJob) -> bool {
    job.state == QueueJobState::Pending
        && job.attempt_count == 0
        && job.execution_generation.is_none()
        && job.turn_id.is_none()
}

fn hold_originals(
    db: &Connection,
    scope: StopScope<'_>,
    jobs: &[StoredQueueJob],
    operation: &str,
) -> Result<Vec<(String, String, String)>> {
    let mut holds = Vec::with_capacity(jobs.len());
    for job in jobs {
        let wanted = hold_snapshot(db, &job.job_id)?.unwrap_or_else(|| {
            (
                scope.target.into(),
                super::super::REASON.into(),
                json!({"kind":"stop","operation_id":operation,"request":job}).to_string(),
            )
        });
        if wanted.0 != scope.target {
            return Err(refused());
        }
        crate::execution_hold::hold_in(db, &job.job_id, scope.target, &wanted.1, &wanted.2)?;
        holds.push(wanted);
    }
    Ok(holds)
}

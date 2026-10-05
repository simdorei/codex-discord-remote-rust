//! A Goal successor needs both the prior typed terminal and an exact owned CAS.
use super::{
    FORMAT_VERSION, MAX_EVIDENCE_BYTES, MAX_TARGET_RECORDS, Obligation, held, read_in, schema,
};
use crate::{
    Result,
    queue::{QueueJobState, StoredQueueJob},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize)]
pub(super) struct ExecutionOwner {
    pub turn_id: String,
    pub generation: i64,
    pub observer: String,
    pub job: Value,
}

#[derive(Serialize, Deserialize)]
struct Handoff {
    version: i64,
    revision: i64,
    claim_sha256: String,
    previous_terminal: String,
    owner: ExecutionOwner,
}

pub(super) fn job_value(job: &StoredQueueJob) -> Result<Value> {
    let mut value = serde_json::to_value(job)?;
    value["created_at"] = job.created_at.to_bits().into();
    value["updated_at"] = job.updated_at.to_bits().into();
    Ok(value)
}

pub(super) fn owner_in(db: &Connection, row: &Obligation) -> Result<ExecutionOwner> {
    let stored: Option<(i64, String, String)> = db
        .query_row(
            "SELECT revision,evidence_json,evidence_sha256 FROM cdr_async_execution_handoffs
         WHERE question_id=? ORDER BY revision DESC LIMIT 1",
            [&row.question_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let owner = if let Some((revision, raw, digest)) = stored {
        if raw.len() > MAX_EVIDENCE_BYTES || hex::encode(Sha256::digest(raw.as_bytes())) != digest {
            return Err(held(&row.thread_id, "invalid Goal ownership evidence"));
        }
        let handoff: Handoff = serde_json::from_str(&raw)?;
        if handoff.version != FORMAT_VERSION
            || handoff.revision != revision
            || revision != row.revision
            || handoff.claim_sha256 != row.claim_sha256
            || handoff.previous_terminal.len() > MAX_EVIDENCE_BYTES
        {
            return Err(held(&row.thread_id, "stale Goal ownership evidence"));
        }
        handoff.owner
    } else {
        let claim: Value = serde_json::from_str(&row.claim)?;
        let seal = row
            .original_seal
            .as_deref()
            .ok_or_else(|| held(&row.thread_id, "missing original preparation"))?;
        if seal.len() > MAX_EVIDENCE_BYTES {
            return Err(held(&row.thread_id, "oversized original preparation"));
        }
        let seal: Value = serde_json::from_str(seal)?;
        ExecutionOwner {
            turn_id: row.turn_id.clone(),
            generation: claim["generation"]
                .as_i64()
                .ok_or_else(|| held(&row.thread_id, "missing original generation"))?,
            observer: claim["runtime_id"]
                .as_str()
                .ok_or_else(|| held(&row.thread_id, "missing original observer"))?
                .into(),
            job: seal
                .pointer("/identity/job")
                .cloned()
                .ok_or_else(|| held(&row.thread_id, "missing sealed owner"))?,
        }
    };
    if owner.generation < 0
        || owner.observer.trim().is_empty()
        || owner.observer.len() > 256
        || owner.turn_id.trim().is_empty()
        || owner.job["job_id"].as_str() != Some(&row.origin_job_id)
        || owner.job["target_thread_id"].as_str() != Some(&row.thread_id)
        || owner.job["turn_id"].as_str() != Some(&owner.turn_id)
    {
        return Err(held(&row.thread_id, "invalid execution owner identity"));
    }
    Ok(owner)
}

fn claim_is_current(db: &Connection, row: &Obligation) -> Result<bool> {
    let actual: Option<(String, Option<String>)> = db
        .query_row(
            &format!(
                "SELECT {},q.preparation_json FROM cdr_async_questions q WHERE q.id=?
         AND EXISTS(SELECT 1 FROM mirror_threads m WHERE m.codex_thread_id=q.thread_id
             AND (m.discord_thread_id=q.channel_id OR m.discord_channel_id=q.channel_id))",
                schema::claim_json("q")
            ),
            [&row.question_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(actual.is_some_and(|(claim, seal)| claim == row.claim && seal == row.original_seal))
}

pub(super) fn exact_owner_in(db: &Connection, row: &Obligation) -> Result<bool> {
    if !claim_is_current(db, row)? {
        return Ok(false);
    }
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?)",
        [&row.origin_job_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    let owner = owner_in(db, row)?;
    let job = crate::queue::select_job(db, &row.origin_job_id)?;
    Ok(job.state == QueueJobState::Running
        && !job.goal_waiting
        && job.completion_evidence_generation() == owner.generation
        && job_value(&job)? == owner.job)
}

/// Called after the existing exact waiting-owner CAS, in the SAME transaction.
/// Failure rolls back the queue update too. Compatibility attachment does not
/// call this hook and cannot manufacture a new execution owner.
pub(crate) fn handoff_owned_in(db: &Connection, previous: &StoredQueueJob) -> Result<()> {
    let next = crate::queue::select_job(db, &previous.job_id)?;
    for row in read_in(db, &previous.target_thread_id)? {
        if row.origin_job_id != previous.job_id || row.execution_state != "unresolved" {
            continue;
        }
        let owner = owner_in(db, &row)?;
        let mut waiting = job_value(previous)?;
        waiting["goal_waiting"] = false.into();
        // Only the waiting transition's timestamp is not in the old seal.
        waiting["updated_at"] = owner.job["updated_at"].clone();
        if !previous.goal_waiting
            || previous.state != QueueJobState::Running
            || previous.turn_id.as_deref() != Some(&owner.turn_id)
            || waiting != owner.job
            || !claim_is_current(db, &row)?
        {
            return Err(held(
                &row.thread_id,
                "Goal handoff lost the original execution owner",
            ));
        }
        let Some((proof, _)) = super::terminal::verified_proof_in(db, &row, &owner)? else {
            return Err(held(
                &row.thread_id,
                "Goal handoff has no exact owned terminal evidence",
            ));
        };
        let mut expected_next = job_value(previous)?;
        expected_next["turn_id"] = serde_json::to_value(&next.turn_id)?;
        expected_next["turn_observation_generation"] =
            serde_json::to_value(next.turn_observation_generation)?;
        expected_next["goal_waiting"] = false.into();
        expected_next["updated_at"] = next.updated_at.to_bits().into();
        if expected_next != job_value(&next)?
            || next.turn_id == previous.turn_id
            || next.state != QueueJobState::Running
            || next.goal_waiting
        {
            return Err(held(
                &row.thread_id,
                "Goal successor snapshot is not an exact owned handoff",
            ));
        }
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM cdr_async_execution_handoffs WHERE question_id=?",
            [&row.question_id],
            |r| r.get(0),
        )?;
        if usize::try_from(count).map_or(true, |count| count >= MAX_TARGET_RECORDS) {
            return Err(held(
                &row.thread_id,
                "Goal evidence chain needs bounded reconciliation",
            ));
        }
        let next_revision = row
            .revision
            .checked_add(1)
            .ok_or_else(|| held(&row.thread_id, "execution revision overflow"))?;
        let handoff = Handoff {
            version: FORMAT_VERSION,
            revision: next_revision,
            claim_sha256: row.claim_sha256.clone(),
            previous_terminal: proof.clone(),
            owner: ExecutionOwner {
                turn_id: next
                    .turn_id
                    .clone()
                    .ok_or_else(|| held(&row.thread_id, "missing Goal successor turn"))?,
                generation: next.completion_evidence_generation(),
                observer: owner.observer,
                job: job_value(&next)?,
            },
        };
        let raw = serde_json::to_string(&handoff)?;
        if raw.len() > MAX_EVIDENCE_BYTES {
            return Err(held(&row.thread_id, "Goal handoff evidence exceeds bound"));
        }
        db.execute("INSERT INTO cdr_async_execution_handoffs(question_id,revision,evidence_json,evidence_sha256) VALUES(?,?,?,?)",
            params![row.question_id,next_revision,raw,hex::encode(Sha256::digest(raw.as_bytes()))])?;
        if db.execute("UPDATE cdr_async_execution_obligations SET revision=?,terminal_proof_json=NULL
            WHERE question_id=? AND revision=? AND terminal_proof_json=? AND execution_state='unresolved'",
            params![next_revision,row.question_id,row.revision,proof])? != 1
        {
            return Err(held(&row.thread_id, "Goal handoff lost the exact policy revision"));
        }
    }
    Ok(())
}

//! Typed notification evidence is retained before the transient journal is erased.
//! Only an exact owned terminal transaction may settle an ordinary obligation.
use super::ownership::{ExecutionOwner, owner_in};
use super::{FORMAT_VERSION, MAX_EVIDENCE_BYTES, Obligation, exact_owner_in, held, read_in};
use crate::{Result, queue::StoredQueueJob};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

// Private construction: callers cannot turn an arbitrary status string into
// settlement authority. Decoding, provenance, claim and CAS are revalidated.
#[derive(Serialize, Deserialize)]
pub(super) struct TerminalEvidence {
    version: i64,
    source: String,
    observer: String,
    generation: i64,
    thread_id: String,
    turn_id: String,
    canonical_terminal: Value,
    payload_sha256: String,
    claim_sha256: String,
    revision: i64,
    owner_verified: bool,
}

fn canonical(thread: &str, turn: &str, payload: &str) -> Result<Value> {
    if payload.len() > MAX_EVIDENCE_BYTES {
        return Err(held(
            thread,
            "terminal evidence exceeds the bounded payload limit",
        ));
    }
    let value: Value = serde_json::from_str(payload)?;
    let status = value.pointer("/turn/status").and_then(Value::as_str);
    if value.get("threadId").and_then(Value::as_str) != Some(thread)
        || value.pointer("/turn/id").and_then(Value::as_str) != Some(turn)
        || !matches!(status, Some("completed" | "failed" | "interrupted"))
    {
        return Err(held(thread, "notification is not the exact typed terminal"));
    }
    Ok(json!({"threadId":thread,"turn":{"id":turn,"status":status}}))
}

fn digest(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

/// Called only from the resident notification journal producer. A candidate
/// without live ownership is retained but is NOT ordinary settlement authority.
pub fn record_terminal_notification(
    path: &Path,
    thread: &str,
    turn: &str,
    generation: i64,
    observer: &str,
    payload: &str,
) -> Result<()> {
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut matching = Vec::new();
    for row in read_in(&tx, thread)? {
        if row.version != FORMAT_VERSION || row.execution_state != "unresolved" {
            continue;
        }
        let owner = owner_in(&tx, &row)?;
        if owner.turn_id == turn {
            matching.push((row, owner));
        }
    }
    if matching.is_empty() {
        return Ok(());
    }
    if observer.trim().is_empty() || observer.len() > 256 || generation < 0 {
        return Err(held(thread, "terminal observer identity is invalid"));
    }
    let metadata = canonical(thread, turn, payload)?;
    let mut conflict = false;
    for (row, owner) in matching {
        conflict |= capture_notification_in(&tx, &row, &owner, &metadata, observer, generation)?;
    }
    tx.commit()?;
    if conflict {
        return Err(held(
            thread,
            "conflicting accepted terminal observations require reconciliation",
        ));
    }
    Ok(())
}

fn proof_text_in(db: &Connection, row: &Obligation) -> Result<Option<String>> {
    let (size, raw): (Option<i64>, Option<String>) = db.query_row(
        "SELECT length(CAST(terminal_proof_json AS BLOB)),
         CASE WHEN length(CAST(terminal_proof_json AS BLOB))<=131072 THEN terminal_proof_json ELSE NULL END
         FROM cdr_async_execution_obligations WHERE question_id=?",
        [&row.question_id], |r| Ok((r.get(0)?,r.get(1)?)),
    )?;
    if size.is_some_and(|size| size > 131_072) {
        return Err(held(
            &row.thread_id,
            "oversized terminal proof must remain preserved",
        ));
    }
    Ok(raw)
}

/// Revalidate the accepted observation inside the raw journal write transaction.
/// Unverified candidates are diagnostic evidence, never resident journal owners.
pub(crate) fn journal_observation_allowed_in(
    db: &Connection,
    thread: &str,
    turn: &str,
    generation: i64,
    observer: &str,
    payload: &str,
) -> Result<bool> {
    for row in read_in(db, thread)? {
        if row.version != FORMAT_VERSION {
            return Ok(false);
        }
        let owner = owner_in(db, &row)?;
        if owner.turn_id != turn {
            continue;
        }
        if !exact_owner_in(db, &row)? {
            return Ok(false);
        }
        let Some((_, evidence)) = verified_proof_in(db, &row, &owner)? else {
            return Ok(false);
        };
        if evidence.observer != observer
            || evidence.generation != generation
            || evidence.canonical_terminal != canonical(thread, turn, payload)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn capture_notification_in(
    db: &Connection,
    row: &Obligation,
    owner: &ExecutionOwner,
    metadata: &Value,
    observer: &str,
    generation: i64,
) -> Result<bool> {
    let owner_verified =
        owner.observer == observer && owner.generation == generation && exact_owner_in(db, row)?;
    let evidence = TerminalEvidence {
        version: FORMAT_VERSION,
        source: "resident_notification_v1".into(),
        observer: observer.into(),
        generation,
        thread_id: row.thread_id.clone(),
        turn_id: owner.turn_id.clone(),
        canonical_terminal: metadata.clone(),
        payload_sha256: digest(metadata)?,
        claim_sha256: row.claim_sha256.clone(),
        revision: row.revision,
        owner_verified,
    };
    let raw = serde_json::to_string(&evidence)?;
    if !owner_verified {
        super::candidates::retain(db, row, "unverified", &raw)?;
        return Ok(false);
    }
    if super::candidates::conflicted(db, row)? {
        return Ok(true);
    }
    let prior = proof_text_in(db, row)?;
    if let Some(previous) = &prior {
        if let Ok(Some(accepted)) = decode_proof(row, owner, previous) {
            if accepted.canonical_terminal == *metadata {
                return Ok(false);
            }
            super::candidates::retain(db, row, "conflict", &raw)?;
            return Ok(true);
        }
        // Preserve the exact old candidate, even if malformed, before promoting
        // a new independently verified notification. Never truncate into proof.
        super::candidates::retain(db, row, "replaced", previous)?;
    }
    if db.execute("UPDATE cdr_async_execution_obligations SET terminal_proof_json=?
        WHERE question_id=? AND revision=? AND execution_state='unresolved' AND terminal_proof_json IS ?",
        params![raw,row.question_id,row.revision,prior])? != 1
    {
        return Err(held(&row.thread_id,"accepted terminal proof lost its exact claim"));
    }
    Ok(false)
}

pub(super) fn verified_proof_in(
    db: &Connection,
    row: &Obligation,
    owner: &ExecutionOwner,
) -> Result<Option<(String, TerminalEvidence)>> {
    if super::candidates::conflicted(db, row)? {
        return Ok(None);
    }
    let raw = proof_text_in(db, row)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    Ok(decode_proof(row, owner, &raw)?.map(|evidence| (raw, evidence)))
}

fn decode_proof(
    row: &Obligation,
    owner: &ExecutionOwner,
    raw: &str,
) -> Result<Option<TerminalEvidence>> {
    if raw.len() > MAX_EVIDENCE_BYTES {
        return Err(held(&row.thread_id, "oversized terminal proof"));
    }
    let evidence: TerminalEvidence = serde_json::from_str(raw)?;
    let metadata = canonical(
        &row.thread_id,
        &owner.turn_id,
        &serde_json::to_string(&evidence.canonical_terminal)?,
    )?;
    if evidence.version != FORMAT_VERSION
        || evidence.source != "resident_notification_v1"
        || !evidence.owner_verified
        || evidence.observer != owner.observer
        || evidence.generation != owner.generation
        || evidence.thread_id != row.thread_id
        || evidence.turn_id != owner.turn_id
        || evidence.claim_sha256 != row.claim_sha256
        || evidence.revision != row.revision
        || evidence.payload_sha256 != digest(&metadata)?
    {
        return Ok(None);
    }
    Ok(Some(evidence))
}

/// Invoked inside the existing owned completion IMMEDIATE transaction, before
/// queue deletion/outbox commit. Compatibility removals never call this hook.
pub(crate) fn settle_owned_in(
    db: &Connection,
    expected: &StoredQueueJob,
    release: Option<(&str, i64)>,
) -> Result<()> {
    let Some((observer, generation)) = release else {
        return Ok(());
    };
    let Some(turn) = expected.turn_id.as_deref() else {
        return Ok(());
    };
    if expected.goal_waiting || expected.completion_evidence_generation() != generation {
        return Ok(());
    }
    let actual = crate::queue::select_job(db, &expected.job_id)?;
    if &actual != expected {
        return Ok(());
    }
    for row in read_in(db, &expected.target_thread_id)? {
        if row.origin_job_id != expected.job_id
            || row.version != FORMAT_VERSION
            || row.execution_state != "unresolved"
            || !exact_owner_in(db, &row)?
        {
            continue;
        }
        let owner = owner_in(db, &row)?;
        if owner.turn_id != turn || owner.observer != observer || owner.generation != generation {
            continue;
        }
        let Some((raw, _)) = verified_proof_in(db, &row, &owner)? else {
            continue;
        };
        let next_revision = row
            .revision
            .checked_add(1)
            .ok_or_else(|| held(&row.thread_id, "terminal revision overflow"))?;
        let changed = db.execute(
            "UPDATE cdr_async_execution_obligations SET execution_state='terminal',
             admission_state=CASE WHEN policy='ordinary' THEN 'settled' ELSE 'held' END,
             answer_state=CASE WHEN answer_state='unresolved' THEN 'terminal_without_receipt' ELSE answer_state END,
             revision=revision+1 WHERE question_id=? AND revision=? AND terminal_proof_json=?
             AND execution_state='unresolved'",
            params![row.question_id,row.revision,raw],
        )?;
        if changed != 1 {
            return Err(held(
                &row.thread_id,
                "terminal settlement lost its exact revision",
            ));
        }
        db.execute("INSERT INTO cdr_async_terminal_settlements(question_id,revision,proof_json) VALUES(?,?,?)",
            params![row.question_id,next_revision,raw])?;
        db.execute(
            "UPDATE cdr_async_questions SET state='closed_unknown' WHERE id=? AND state='dispatching'
             AND dispatch_mode='steer' AND preparation_json=?",
            params![row.question_id,row.original_seal],
        )?;
    }
    Ok(())
}

pub(crate) fn retain_terminal_journal(path: &Path, thread: &str, turn: &str) -> Result<bool> {
    let db = crate::schema::open_initialized(path)?;
    retain_terminal_journal_in(&db, thread, turn)
}

pub(crate) fn retain_terminal_journal_in(
    db: &Connection,
    thread: &str,
    turn: &str,
) -> Result<bool> {
    let journal: Option<(i64, Option<String>, Option<String>)> = db.query_row(
        "SELECT generation,CASE WHEN length(CAST(payload AS BLOB))<=131072 THEN payload ELSE NULL END,resident_owner
         FROM codex_observed_completions WHERE thread_id=? AND turn_id=?",
        params![thread,turn], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let Some((generation, payload, resident)) = journal else {
        return Ok(false);
    };
    for row in read_in(db, thread)? {
        if row.execution_state != "unresolved" {
            continue;
        }
        let Ok(owner) = owner_in(db, &row) else {
            return Ok(true);
        };
        if owner.turn_id != turn {
            continue;
        }
        let Ok(Some((_, evidence))) = verified_proof_in(db, &row, &owner) else {
            return Ok(true);
        };
        let Some(payload) = payload.as_deref() else {
            return Ok(true);
        };
        let Ok(metadata) = canonical(thread, turn, payload) else {
            return Ok(true);
        };
        if generation != evidence.generation
            || resident.as_deref() != Some(evidence.observer.as_str())
            || metadata != evidence.canonical_terminal
        {
            return Ok(true);
        }
    }
    Ok(false)
}

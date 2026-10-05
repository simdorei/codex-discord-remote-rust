//! Historical logical-execution authority, distinct from resident notifications.
use super::super::{MAX_EVIDENCE_BYTES, held, ownership, schema, terminal};
use super::{HistorySnapshot, hash, original_question};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Serialize)]
struct Entry {
    index: usize,
    owner: ownership::ExecutionOwner,
    prior_proof: Option<String>,
    source_state: String,
    source_error: Option<String>,
}

/// Private construction binds an observation to its original immutable owner.
pub struct TerminalHistorySnapshot {
    history: HistorySnapshot,
    entries: Vec<Entry>,
}

impl TerminalHistorySnapshot {
    #[must_use]
    pub fn turn_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|e| e.owner.turn_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

fn entry_in(db: &Connection, history: &HistorySnapshot, index: usize) -> Result<Entry> {
    let row = &history.rows[index];
    let q = original_question(row)?;
    if !matches!(row.policy.as_str(), "ordinary" | "publishing_recovery") {
        return Err(held(
            &history.thread,
            "unsupported historical execution policy",
        ));
    }
    let mapped:bool=db.query_row(
        "SELECT COUNT(*)=1 AND MIN(codex_thread_id)=?2 FROM mirror_threads WHERE discord_thread_id=?1",
        params![q.channel_id,q.thread_id],|r|r.get(0),
    )?;
    let current:Option<(String,Option<String>,String,Option<String>)>=db.query_row(
        &format!("SELECT {},q.preparation_json,q.state,q.error FROM cdr_async_questions q WHERE q.id=?",
            schema::claim_json("q")),
        [&q.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional()?;
    let Some((claim, seal, state, error)) = current else {
        return Err(held(
            &history.thread,
            "historical original question is missing",
        ));
    };
    let conflict: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_terminal_candidates
         WHERE question_id=? AND revision=? AND
         (kind='conflict' OR CASE WHEN json_valid(evidence_text) THEN
            json_extract(evidence_text,'$.answer_conflict')=1 ELSE 0 END))",
        params![q.id, row.revision],
        |r| r.get(0),
    )?;
    if !mapped
        || claim != row.claim
        || seal != row.original_seal
        || !matches!(state.as_str(), "dispatching" | "submitted")
        || conflict
    {
        return Err(held(
            &history.thread,
            "historical original identity or evidence conflicts",
        ));
    }
    let (size, prior): (Option<i64>, Option<String>) = db.query_row(
        "SELECT length(CAST(terminal_proof_json AS BLOB)),
         CASE WHEN length(CAST(terminal_proof_json AS BLOB))<=131072 THEN terminal_proof_json END
         FROM cdr_async_execution_obligations WHERE question_id=?",
        [&q.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if size.is_some_and(|n| n > 131_072) {
        return Err(held(
            &history.thread,
            "prior terminal evidence exceeds the preservation bound",
        ));
    }
    Ok(Entry {
        index,
        owner: ownership::owner_in(db, row)?,
        prior_proof: prior,
        source_state: state,
        source_error: error,
    })
}

fn capture_in(db: &Connection, thread: &str) -> Result<Option<TerminalHistorySnapshot>> {
    let history = super::snapshot_in(db, thread)?;
    let active:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE target_thread_id=? AND state!='pending')",
        [thread],|r|r.get(0),
    )?;
    if active {
        return Ok(None);
    }
    let mut entries = Vec::new();
    for (index, row) in history.rows.iter().enumerate() {
        if row.execution_state != "unresolved" {
            continue;
        }
        let present: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?)",
            [&row.origin_job_id],
            |r| r.get(0),
        )?;
        if present {
            return Ok(None);
        }
        entries.push(entry_in(db, &history, index)?);
    }
    if entries.is_empty() {
        return Ok(None);
    }
    Ok(Some(TerminalHistorySnapshot { history, entries }))
}

pub fn capture_terminal_history_snapshot(
    path: &Path,
    thread: &str,
) -> Result<Option<TerminalHistorySnapshot>> {
    let db = crate::schema::open_initialized(path)?;
    let tx = db.unchecked_transaction()?;
    capture_in(&tx, thread)
}

fn fingerprint(snapshot: &TerminalHistorySnapshot) -> Result<String> {
    Ok(serde_json::to_string(&(
        super::fingerprint(&snapshot.history)?,
        &snapshot.entries,
    ))?)
}

fn observations<'a>(
    expected: &TerminalHistorySnapshot,
    observed: &'a Value,
) -> Result<&'a [Value]> {
    let thread = &expected.history.thread;
    let invalid = || {
        held(
            thread,
            "historical execution needs exact idle thread and explicit ended Goal",
        )
    };
    let metadata = &observed["thread_observation"];
    let goal = observed
        .get("goal_observation")
        .and_then(|v| v.get("goal"))
        .ok_or_else(invalid)?;
    if observed.to_string().len() > 1_048_576
        || observed["threadId"] != *thread
        || observed["truncated"] == true
        || metadata["thread"]["id"] != *thread
        || metadata["thread"]["status"]["type"] != "idle"
        || !(goal.is_null() || (goal["threadId"] == *thread && goal["status"] == "complete"))
    {
        return Err(invalid());
    }
    let turns = observed
        .get("turns")
        .and_then(Value::as_array)
        .filter(|v| v.len() <= 128)
        .ok_or_else(invalid)?;
    let mut ids = std::collections::BTreeSet::new();
    for turn in turns {
        let id = turn
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
            .ok_or_else(invalid)?;
        if !ids.insert(id) {
            return Err(invalid());
        }
    }
    Ok(turns)
}

fn proof_in(
    db: &Connection,
    expected: &TerminalHistorySnapshot,
    entry: &Entry,
    observed: &Value,
    turns: &[Value],
    reader: &str,
    generation: i64,
) -> Result<String> {
    let row = &expected.history.rows[entry.index];
    let invalid = || {
        held(
            &row.thread_id,
            "historical current logical owner lacks exact terminal evidence",
        )
    };
    let turn = turns
        .iter()
        .find(|t| t["id"] == entry.owner.turn_id)
        .ok_or_else(invalid)?;
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "completed" | "failed" | "interrupted"))
        .ok_or_else(invalid)?;
    if let Some(prior) = &entry.prior_proof {
        let Some((verified, _)) = terminal::verified_proof_in(db, row, &entry.owner)? else {
            return Err(invalid());
        };
        let old: Value = serde_json::from_str(&verified)?;
        if verified != *prior || old["canonical_terminal"]["turn"]["status"] != status {
            return Err(invalid());
        }
    }
    let proof=json!({
        "version":1,"source":"historical_read_terminal_v1",
        "observer":reader,"generation":generation,"owner_verified":true,
        "question_id":row.question_id,"claim_sha256":row.claim_sha256,"revision":row.revision,
        "thread_id":row.thread_id,"turn_id":entry.owner.turn_id,"sealed_execution_owner":entry.owner,
        "canonical_terminal":{"threadId":row.thread_id,"turn":{"id":entry.owner.turn_id,"status":status}},
        "goal":observed["goal_observation"]["goal"],
        "history_sha256":hash(&observed.to_string()),"owner_turn_sha256":hash(&turn.to_string()),
        "thread_observation_sha256":hash(&observed["thread_observation"].to_string()),
        "previous_terminal_proof":entry.prior_proof
    }).to_string();
    if proof.len() > MAX_EVIDENCE_BYTES {
        return Err(held(
            &row.thread_id,
            "historical proof exceeds preservation bound",
        ));
    }
    Ok(proof)
}

fn settle_one_in(
    db: &Connection,
    expected: &TerminalHistorySnapshot,
    entry: &Entry,
    proof: &str,
) -> Result<()> {
    let row = &expected.history.rows[entry.index];
    let invalid = || {
        held(
            &row.thread_id,
            "historical settlement lost its exact atomic certificate",
        )
    };
    let next = row.revision.checked_add(1).ok_or_else(invalid)?;
    let admission = if row.policy == "ordinary" {
        "settled"
    } else {
        "held"
    };
    let answer = if row.answer_state == "unresolved" {
        "terminal_without_receipt"
    } else {
        &row.answer_state
    };
    if db.execute(
        "UPDATE cdr_async_execution_obligations SET execution_state='terminal',admission_state=?,
         answer_state=?,revision=?,terminal_proof_json=?
         WHERE question_id=? AND revision=? AND execution_state='unresolved' AND terminal_proof_json IS ?",
        params![admission,answer,next,proof,row.question_id,row.revision,entry.prior_proof],
    )?!=1 {return Err(invalid());}
    if db.execute(
        "INSERT INTO cdr_async_terminal_settlements(question_id,revision,proof_json) VALUES(?,?,?)",
        params![row.question_id, next, proof],
    )? != 1
    {
        return Err(invalid());
    }
    if entry.source_state == "dispatching"
        && db.execute(
            "UPDATE cdr_async_questions SET state='closed_unknown'
         WHERE id=? AND state='dispatching' AND dispatch_mode='steer' AND preparation_json=?",
            params![row.question_id, row.original_seal],
        )? != 1
    {
        return Err(invalid());
    }
    let saved:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_execution_obligations o
         JOIN cdr_async_terminal_settlements s ON s.question_id=o.question_id
         WHERE o.question_id=? AND o.revision=? AND s.revision=o.revision
         AND o.terminal_proof_json=? AND s.proof_json=o.terminal_proof_json
         AND o.execution_state='terminal' AND o.policy=? AND o.admission_state=? AND o.answer_state=?)",
        params![row.question_id,next,proof,row.policy,admission,answer],|r|r.get(0),
    )?;
    if !saved {
        return Err(invalid());
    }
    Ok(())
}

fn verify_sources_in(db: &Connection, expected: &TerminalHistorySnapshot) -> Result<()> {
    for entry in &expected.entries {
        let row = &expected.history.rows[entry.index];
        let state = if entry.source_state == "dispatching" {
            "closed_unknown"
        } else {
            "submitted"
        };
        let intact: bool = db.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM cdr_async_questions q WHERE q.id=?
             AND q.state=? AND {}=? AND q.preparation_json IS ? AND q.error IS ?)",
                schema::claim_json("q")
            ),
            params![
                row.question_id,
                state,
                row.claim,
                row.original_seal,
                entry.source_error
            ],
            |r| r.get(0),
        )?;
        if !intact {
            return Err(held(
                &row.thread_id,
                "historical settlement changed its original question",
            ));
        }
    }
    Ok(())
}

/// The owning runtime supplies fresh read-only observations under its target lock
/// and control permit. This never requeues, dispatches, or releases other holds.
pub fn settle_terminal_history(
    path: &Path,
    expected: &TerminalHistorySnapshot,
    observed: &Value,
    reader: &str,
    generation: i64,
) -> Result<usize> {
    if reader.trim().is_empty() || reader.len() > 256 || generation < 0 {
        return Err(held(
            &expected.history.thread,
            "historical reader identity is invalid",
        ));
    }
    let turns = observations(expected, observed)?;
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(current) = capture_in(&tx, &expected.history.thread)? else {
        return Err(held(
            &expected.history.thread,
            "historical execution owner changed while reading",
        ));
    };
    if fingerprint(&current)? != fingerprint(expected)? {
        return Err(held(
            &expected.history.thread,
            "historical execution snapshot changed while reading",
        ));
    }
    for entry in &expected.entries {
        let proof = proof_in(&tx, expected, entry, observed, turns, reader, generation)?;
        settle_one_in(&tx, expected, entry, &proof)?;
    }
    verify_sources_in(&tx, expected)?;
    tx.commit()?;
    Ok(expected.entries.len())
}

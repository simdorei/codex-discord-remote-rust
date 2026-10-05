//! An answer receipt is not proof that the originating execution terminated.
//!
//! This ledger survives queue deletion and question retention. No API in this
//! module requeues a job, repeats an answer, or infers termination from absence.
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Result, StoreError};

pub mod abandonment;
pub mod admission_order;
mod candidates;
mod lifecycle;
mod ownership;
mod policy;
pub mod publication;
mod schema;
mod terminal;
pub use policy::{
    RECOVERY_POLICY_COMPONENT, RECOVERY_POLICY_FORMAT_VERSION, REVIEWED_INCIDENT_THREAD,
    REVIEWED_PROPOSAL_SHA256, install_reviewed_policy, reviewed_policy_installed_in,
};
mod preflight;
pub(crate) use preflight::unprovable_in;
mod history;
pub use history::{HistorySnapshot, capture_history_snapshot, retain_history_candidate};
pub use history::{
    TerminalHistorySnapshot, capture_terminal_history_snapshot, settle_terminal_history,
};
pub(crate) use ownership::handoff_owned_in;
pub(crate) use schema::{migrate_schema, schema_current};
pub(crate) use terminal::journal_observation_allowed_in;
pub use terminal::record_terminal_notification;
pub(crate) use terminal::{retain_terminal_journal, retain_terminal_journal_in, settle_owned_in};

pub const HOLD_PREFIX: &str = "[cdr-rust:async-resolution-held:v1] ";
pub const FORMAT_VERSION: i64 = 1;
const MAX_TARGET_RECORDS: usize = 128;
const MAX_EVIDENCE_BYTES: usize = 131_072;

#[derive(Debug, Serialize)]
pub struct Obligation {
    pub question_id: String,
    pub thread_id: String,
    pub origin_job_id: String,
    pub turn_id: String,
    pub version: i64,
    pub revision: i64,
    pub answer_state: String,
    pub execution_state: String,
    pub admission_state: String,
    pub policy: String,
    pub claim_sha256: String,
    #[serde(skip)]
    original_seal: Option<String>,
    #[serde(skip)]
    claim: String,
    pub original_error: String,
}

fn held(thread: &str, reason: &str) -> StoreError {
    StoreError::AsyncResolutionHeld {
        thread_id: thread.to_owned(),
        reason: reason.to_owned(),
    }
}

fn has_table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?)",
        [name],
        |r| r.get(0),
    )?)
}

/// Read-only compatibility predicate. A missing ledger never hides a legacy
/// dispatching question; a malformed present schema is an error, not permission.
pub fn held_in(db: &Connection, thread: &str) -> Result<bool> {
    if policy::held_in(db, thread)? {
        return Ok(true);
    }
    if has_table(db, "cdr_async_execution_obligations")? {
        let held: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations WHERE thread_id=?)",
            [thread],
            |r| r.get(0),
        )?;
        if held {
            return Ok(true);
        }
        if lifecycle::admission_held_in(db, thread)? {
            return Ok(true);
        }
    }
    if !has_table(db, "cdr_async_questions")? {
        return Ok(false);
    }
    let recorded = if has_table(db, "cdr_async_execution_obligations")? {
        " AND NOT EXISTS(SELECT 1 FROM cdr_async_execution_obligations o WHERE o.question_id=q.id)"
    } else {
        ""
    };
    Ok(db.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM cdr_async_questions q WHERE q.thread_id=?
         AND q.state='dispatching' AND q.dispatch_mode='steer'{recorded})"
        ),
        [thread],
        |r| r.get(0),
    )?)
}

pub fn admission_held(path: &Path, thread: &str) -> Result<bool> {
    held_in(&crate::schema::open_initialized(path)?, thread)
}

pub fn assert_admission_in(db: &Connection, thread: &str) -> Result<()> {
    if held_in(db, thread)? {
        return Err(held(
            thread,
            "original async execution, lifecycle request or recovery authorization is unresolved; no automatic retry",
        ));
    }
    Ok(())
}

pub(crate) fn cleanup_held_in(db: &Connection, channel: i64, thread: Option<&str>) -> Result<bool> {
    if let Some(thread) = thread
        && held_in(db, thread)?
    {
        return Ok(true);
    }
    if !has_table(db, "cdr_async_execution_obligations")? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_async_unsettled_obligations WHERE channel_id=?)",
        [channel],
        |r| r.get(0),
    )?)
}

/// The still-owned Running execution may finish its existing lifecycle. A lost,
/// changed, or unprovable owner cannot borrow that exception for a new mutation.
pub fn guard_mutation(path: &Path, thread: &str) -> Result<()> {
    let db = crate::schema::open_initialized(path)?;
    let tx = db.unchecked_transaction()?;
    guard_mutation_in(&tx, thread)
}

pub(crate) fn guard_mutation_in(db: &Connection, thread: &str) -> Result<()> {
    if policy::held_in(db, thread)? {
        return Err(held(
            thread,
            "reviewed publishing recovery policy remains held; no unscoped mutation or replay",
        ));
    }
    for row in read_in(db, thread)? {
        if row.version != FORMAT_VERSION
            || row.revision < 0
            || row.claim.len() > MAX_EVIDENCE_BYTES
            || row.original_seal.as_ref().is_none_or(|seal| {
                seal.len() > MAX_EVIDENCE_BYTES
                    || serde_json::from_str::<serde_json::Value>(seal).map_or(true, |value| {
                        !value.is_object()
                            || value.as_object().is_none_or(serde_json::Map::is_empty)
                    })
            })
        {
            return Err(held(
                thread,
                "invalid or unsupported original claim evidence",
            ));
        }
        if row.policy != "ordinary"
            || row.execution_state != "unresolved"
            || row.admission_state != "held"
            || !exact_owner_in(db, &row)?
        {
            return Err(held(
                thread,
                "originating execution has no exact live owner; terminal reconciliation required; no automatic retry",
            ));
        }
    }
    Ok(())
}

pub(crate) fn certified_successor_in(
    db: &Connection,
    thread: &str,
    question: &str,
) -> Result<bool> {
    let Some(row) = read_in(db, thread)?
        .into_iter()
        .find(|row| row.question_id == question)
    else {
        return Ok(false);
    };
    if row.version != FORMAT_VERSION
        || row.policy != "ordinary"
        || row.execution_state != "unresolved"
        || row.admission_state != "held"
    {
        return Ok(false);
    }
    let certified: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM cdr_async_execution_handoffs WHERE question_id=? AND revision=?)",
        rusqlite::params![question,row.revision], |r| r.get(0))?;
    Ok(certified && exact_owner_in(db, &row)?)
}

fn exact_owner_in(db: &Connection, row: &Obligation) -> Result<bool> {
    ownership::exact_owner_in(db, row)
}

fn read_in(db: &Connection, thread: &str) -> Result<Vec<Obligation>> {
    read_records_in(db, thread, true)
}

fn read_all_in(db: &Connection, thread: &str) -> Result<Vec<Obligation>> {
    read_records_in(db, thread, false)
}

fn read_records_in(db: &Connection, thread: &str, unsettled: bool) -> Result<Vec<Obligation>> {
    if !has_table(db, "cdr_async_execution_obligations")? {
        return Ok(Vec::new());
    }
    let source = if unsettled {
        "cdr_async_unsettled_obligations"
    } else {
        "cdr_async_execution_obligations"
    };
    let mut statement = db.prepare(&format!(
        "SELECT question_id,thread_id,origin_job_id,turn_id,format_version,revision,
         answer_state,execution_state,admission_state,policy,original_seal,claim_json,
         owner_json,original_error FROM {source} WHERE thread_id=?
         ORDER BY question_id LIMIT 129",
    ))?;
    let rows = statement
        .query_map([thread], |r| {
            let claim: String = r.get(11)?;
            let seal: Option<String> = r.get(10)?;
            let owner: Option<String> = r.get(12)?;
            // Length-prefixed domains avoid concatenation ambiguity. The underlying
            // exact bytes are immutable; the digest is derived, never an authority.
            let mut hash = Sha256::new();
            for value in [Some(claim.as_str()), seal.as_deref(), owner.as_deref()] {
                match value {
                    Some(value) => {
                        hash.update([1]);
                        hash.update((value.len() as u64).to_le_bytes());
                        hash.update(value.as_bytes());
                    }
                    None => hash.update([0]),
                }
            }
            Ok(Obligation {
                question_id: r.get(0)?,
                thread_id: r.get(1)?,
                origin_job_id: r.get(2)?,
                turn_id: r.get(3)?,
                version: r.get(4)?,
                revision: r.get(5)?,
                answer_state: r.get(6)?,
                execution_state: r.get(7)?,
                admission_state: r.get(8)?,
                policy: r.get(9)?,
                claim_sha256: hex::encode(hash.finalize()),
                original_seal: seal,
                claim,
                original_error: r.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > MAX_TARGET_RECORDS {
        return Err(held(
            thread,
            "target evidence page exceeds bounded review limit",
        ));
    }
    Ok(rows)
}

/// Diagnostics must not migrate, backfill, initialize, or manufacture authority.
pub fn inspect(path: &Path, thread: &str) -> Result<Vec<Obligation>> {
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.pragma_update(None, "query_only", true)?;
    let tx = db.unchecked_transaction()?;
    read_in(&tx, thread)
}

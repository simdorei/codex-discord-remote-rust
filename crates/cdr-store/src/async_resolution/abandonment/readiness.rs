//! Read-only prerequisite evidence, never a release or execution permit.
//!
//! Callers must obtain fresh observations under their target/connection fence.
//! This report cannot replace authenticated release consent, historical control
//! disposition, an atomic ingress boundary, or the normal one-use writer.
use super::{api, decision, invalid, proposal};
use crate::Result;
use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

mod context;
mod proof;

pub struct Snapshot {
    thread: String,
    turns: Vec<String>,
    disposition: String,
    revision: i64,
    evidence: Value,
    executions: Vec<proof::Execution>,
}

impl Snapshot {
    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread
    }
    #[must_use]
    pub fn turn_ids(&self) -> Vec<String> {
        self.turns.clone()
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    thread_id: String,
    disposition_id: String,
    disposition_revision: i64,
    evidence_sha256: String,
    observer: String,
    generation: i64,
    release_authorized: bool,
    execution_authorized: bool,
}

fn capture_in(db: &Connection, path: &Path, id: &str, revision: i64) -> Result<Snapshot> {
    if db.is_autocommit() {
        return Err(crate::StoreError::ActiveTransaction);
    }
    let stored = proposal::read_in(db, id)?;
    api::require_database(path, &stored)?;
    let p = &stored.proposal;
    let receipt = decision::receipt_in(db, &stored)?
        .filter(|receipt| receipt.decision == api::Decision::AbandonOnly)
        .ok_or_else(|| invalid("exact completed abandonment is required"))?;
    if revision < 1 || p.revision != revision || proposal::latest_in(db, &p.job_id)? != revision {
        return Err(invalid("disposition revision is not current"));
    }
    let policy = context::policy_in(db, p)?;
    let executions = proof::capture_in(db, p, &policy)?;
    let current = context::capture_in(db, p)?;
    let evidence = json!({"abandonment":stored,"receipt":receipt,"policy":policy,
        "current":current,"executions":executions});
    if serde_json::to_vec(&evidence)?.len() > 1_048_576 {
        return Err(invalid("combined private evidence exceeds the bound"));
    }
    let turns = executions
        .iter()
        .map(|execution| execution.turn_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(Snapshot {
        thread: p.thread_id.clone(),
        turns,
        disposition: id.into(),
        revision,
        evidence,
        executions,
    })
}

/// A bounded read snapshot only. No migration, state write, or permission grant.
pub fn capture(path: &Path, disposition: &str, revision: i64) -> Result<Snapshot> {
    let mut db = api::open(path, true)?;
    db.pragma_update(None, "query_only", true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let snapshot = capture_in(&tx, path, disposition, revision)?;
    tx.commit()?;
    Ok(snapshot)
}

fn observation(
    expected: &Snapshot,
    observed: &Value,
    reader: &str,
    generation: i64,
) -> Result<Value> {
    let invalid_observation =
        || invalid("exact current idle thread, ended Goal and terminal owners are required");
    let thread = &expected.thread;
    let goal = observed
        .get("goal_observation")
        .and_then(|value| value.get("goal"))
        .ok_or_else(invalid_observation)?;
    if observed.to_string().len() > 1_048_576
        || observed["threadId"] != *thread
        || observed["truncated"] != false
        || observed["thread_observation"]["thread"]["id"] != *thread
        || observed["thread_observation"]["thread"]["status"]["type"] != "idle"
        || observed["thread_observation"]["thread"]["archived"] == true
        || !(goal.is_null() || (goal["threadId"] == *thread && goal["status"] == "complete"))
        || generation < 1
        || reader.trim().is_empty()
        || reader.len() > 256
        || expected.evidence["current"]["runtime"]["app"] != reader
    {
        return Err(invalid_observation());
    }
    let turns = observed
        .get("turns")
        .and_then(Value::as_array)
        .filter(|turns| turns.len() <= 128)
        .ok_or_else(invalid_observation)?;
    let mut ids = std::collections::BTreeSet::new();
    for turn in turns {
        let id = turn
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512)
            .ok_or_else(invalid_observation)?;
        if !ids.insert(id)
            || !matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            )
        {
            return Err(invalid_observation());
        }
    }
    for execution in &expected.executions {
        if !turns
            .iter()
            .any(|turn| turn["id"] == execution.turn_id && turn["status"] == execution.status)
        {
            return Err(invalid_observation());
        }
    }
    Ok(
        json!({"thread":thread,"status":"idle","goal":goal,"executions":expected.executions,
        "observer":reader,"generation":generation,"observation_sha256":api::digest(&observed.to_string())}),
    )
}

/// Compare local facts again after the caller's fresh read. Even a successful
/// result is diagnostic only and must never be passed as dispatch authority.
pub fn verify_observation(
    path: &Path,
    expected: &Snapshot,
    observed: &Value,
    reader: &str,
    generation: i64,
) -> Result<Report> {
    let observed = observation(expected, observed, reader, generation)?;
    let mut db = api::open(path, true)?;
    db.pragma_update(None, "query_only", true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let current = capture_in(&tx, path, &expected.disposition, expected.revision)?;
    if current.evidence != expected.evidence {
        return Err(invalid("local evidence changed during the observation"));
    }
    let digest = api::digest(&serde_json::to_string(
        &json!({"local":current.evidence,"observed":observed}),
    )?);
    tx.commit()?;
    Ok(Report {
        thread_id: expected.thread.clone(),
        disposition_id: expected.disposition.clone(),
        disposition_revision: expected.revision,
        evidence_sha256: digest,
        observer: reader.into(),
        generation,
        release_authorized: false,
        execution_authorized: false,
    })
}

//! Reuse the original seal/owner rules at the pre-settlement revision.
use super::{
    super::{api, invalid},
    context::Policy,
};
use crate::{Result, async_resolution as resolution};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
pub(super) struct Execution {
    question_id: String,
    pub(super) turn_id: String,
    pub(super) status: String,
    settlement_revision: i64,
    proof_sha256: String,
}

fn original_is_current(
    db: &Connection,
    row: &resolution::Obligation,
    channel: i64,
) -> Result<bool> {
    Ok(db.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM cdr_async_questions q WHERE q.id=?1
         AND q.channel_id=?2 AND q.state IN ('closed_unknown','submitted')
         AND {}=?3 AND q.preparation_json IS ?4)
         AND NOT EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?5)",
            resolution::schema::claim_json("q")
        ),
        params![
            row.question_id,
            channel,
            row.claim,
            row.original_seal,
            row.origin_job_id
        ],
        |r| r.get(0),
    )?)
}

fn certificate_in(db: &Connection, row: &resolution::Obligation) -> Result<String> {
    Ok(db.query_row(
        "SELECT s.proof_json FROM cdr_async_terminal_settlements s
         JOIN cdr_async_execution_obligations o ON o.question_id=s.question_id
         WHERE s.question_id=? AND s.revision=? AND o.revision=s.revision
         AND o.terminal_proof_json=s.proof_json AND length(CAST(s.proof_json AS BLOB))<=131072",
        params![row.question_id, row.revision],
        |r| r.get(0),
    )?)
}

fn digest(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    })
}

fn historical(
    proof: &Value,
    row: &resolution::Obligation,
    owner: &resolution::ownership::ExecutionOwner,
) -> Result<bool> {
    let goal = proof.get("goal");
    let previous = proof.get("previous_terminal_proof");
    Ok(proof["question_id"] == row.question_id
        && proof["sealed_execution_owner"] == serde_json::to_value(owner)?
        && proof["observer"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty() && value.len() <= 256)
        && proof["generation"].as_i64().is_some_and(|value| value >= 0)
        && goal.is_some_and(|goal| {
            goal.is_null() || (goal["threadId"] == row.thread_id && goal["status"] == "complete")
        })
        && previous.is_some_and(|value| {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|value| value.len() <= resolution::MAX_EVIDENCE_BYTES)
        })
        && [
            "history_sha256",
            "owner_turn_sha256",
            "thread_observation_sha256",
        ]
        .iter()
        .all(|key| digest(&proof[key])))
}

pub(super) fn capture_in(
    db: &Connection,
    proposal: &api::Proposal,
    policy: &Policy,
) -> Result<Vec<Execution>> {
    let rows = resolution::read_all_in(db, &proposal.thread_id)?;
    let mut found_original = false;
    let mut executions = Vec::new();
    for mut row in rows {
        let rejected = || invalid("original execution has no exact supported terminal certificate");
        let revision = row.revision;
        if row.version != resolution::FORMAT_VERSION
            || revision < 1
            || row.execution_state != "terminal"
            || !matches!(
                (row.policy.as_str(), row.admission_state.as_str()),
                ("ordinary", "settled") | ("publishing_recovery", "held")
            )
            || !original_is_current(db, &row, proposal.channel_id)?
            || resolution::candidates::conflicted(db, &row)?
        {
            return Err(rejected());
        }
        let original = resolution::history::original_question(&row)?;
        if row.origin_job_id == policy.origin_job && row.turn_id == policy.original_turn {
            if original.owner_user_id != proposal.owner_user_id {
                return Err(rejected());
            }
            found_original = true;
        }
        let raw = certificate_in(db, &row)?;
        let evidence: Value = serde_json::from_str(&raw)?;
        row.revision = revision - 1;
        let owner = resolution::ownership::owner_in(db, &row)?;
        let status = evidence["canonical_terminal"]["turn"]["status"]
            .as_str()
            .filter(|status| matches!(*status, "completed" | "failed" | "interrupted"))
            .ok_or_else(rejected)?;
        if evidence["version"] != resolution::FORMAT_VERSION
            || evidence["owner_verified"] != true
            || evidence["revision"] != row.revision
            || evidence["claim_sha256"] != row.claim_sha256
            || evidence["thread_id"] != row.thread_id
            || evidence["turn_id"] != owner.turn_id
            || evidence["canonical_terminal"]
                != json!({"threadId":row.thread_id,"turn":{"id":owner.turn_id,"status":status}})
            || resolution::candidates::conflicted(db, &row)?
        {
            return Err(rejected());
        }
        match evidence["source"].as_str() {
            Some("resident_notification_v1") => {
                let verified = resolution::terminal::verified_proof_in(db, &row, &owner)?;
                if verified.is_none_or(|(verified, _)| verified != raw) {
                    return Err(rejected());
                }
            }
            Some("historical_read_terminal_v1") if historical(&evidence, &row, &owner)? => {}
            _ => return Err(rejected()),
        }
        executions.push(Execution {
            question_id: row.question_id,
            turn_id: owner.turn_id,
            status: status.into(),
            settlement_revision: revision,
            proof_sha256: api::digest(&raw),
        });
    }
    if !found_original {
        return Err(invalid("registered original execution evidence is missing"));
    }
    Ok(executions)
}

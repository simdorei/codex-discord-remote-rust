use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

use super::{Proposal, StoredProposal, invalid, read_in, verify_fresh_in};
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    ApproveExact,
    KeepHeld,
}

impl Decision {
    fn stored(self) -> &'static str {
        match self {
            Self::ApproveExact => "approve_exact",
            Self::KeepHeld => "keep_held",
        }
    }
}

pub struct ConsentInput<'a> {
    pub proposal_id: &'a str,
    pub revision: i64,
    pub ingress_id: &'a str,
    pub now: f64,
}

/// This receipt is evidence of intent, not a start token. It is deliberately not
/// convertible to a dispatch permit. Duplicate clicks never create a new row.
#[derive(Debug, PartialEq, Eq)]
pub struct DecisionReceipt {
    pub decision: Decision,
    pub original_ingress_id: String,
    pub original_interaction_id: i64,
    pub already_recorded: bool,
}

struct Click {
    event: i64,
    decision: Decision,
    identity: Value,
}

fn click_in(db: &Connection, input: &ConsentInput<'_>, p: &Proposal) -> Result<Click> {
    let (headers,payload): (Value,String) = db.query_row(
        "SELECT version,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
         target_thread_id,state,phase,runtime_id,owner_kind,owner_id,payload_json
         FROM discord_ingress_journal WHERE ingress_id=?
         AND length(CAST(payload_json AS BLOB))<=131072", [input.ingress_id],
        |r| Ok((json!({
            "version":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,
            "event":r.get::<_,Option<i64>>(2)?,"app":r.get::<_,Option<i64>>(3)?,
            "channel":r.get::<_,i64>(4)?,"owner":r.get::<_,i64>(5)?,
            "message":r.get::<_,Option<i64>>(6)?,"target":r.get::<_,Option<String>>(7)?,
            "state":r.get::<_,String>(8)?,"phase":r.get::<_,String>(9)?,
            "runtime":r.get::<_,Option<String>>(10)?,
            "owner_kind":r.get::<_,Option<String>>(11)?,"owner_id":r.get::<_,Option<String>>(12)?,
        }),r.get(13)?)),
    )?;
    let message: i64 = db.query_row(
        "SELECT message_id FROM cdr_recovery_publication_deliveries
         WHERE proposal_id=? AND revision=? AND body_sha256=?",
        params![p.id, p.revision, p.review_sha256],
        |r| r.get(0),
    )?;
    let payload: Value = serde_json::from_str(&payload)?;
    let decision = match payload
        .pointer("/work/Component/RecoveryPublicationDecision/decision")
        .and_then(Value::as_str)
    {
        Some("ApproveExact") => Decision::ApproveExact,
        Some("KeepHeld") => Decision::KeepHeld,
        _ => return Err(invalid("not an exact publication decision component")),
    };
    let component = json!({"RecoveryPublicationDecision":{
        "proposal_id":p.id,"revision":p.revision,
        "decision":if decision==Decision::ApproveExact {"ApproveExact"} else {"KeepHeld"},
    }});
    if headers["version"] != 1
        || headers["kind"] != "interaction"
        || headers["event"].as_i64().is_none_or(|v| v <= 0)
        || headers["app"] != p.application_id
        || headers["channel"] != p.channel_id
        || headers["owner"] != p.owner_user_id
        || headers["message"] != message
        || headers["target"] != p.thread_id
        || payload["version"] != 1
        || payload["work"] != json!({"Component":component})
        || headers["runtime"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(invalid(
            "actor, application, source message or component binding changed",
        ));
    }
    let event = headers["event"]
        .as_i64()
        .ok_or_else(|| invalid("interaction ID missing"))?;
    // An exact replay of a completed ingress may read its original receipt, but
    // cannot establish new intent from a held/completed/owned ingress.
    let existing = receipt_in(db, &p.id)?;
    let exact_replay = existing.as_ref().is_some_and(|r| {
        r.original_ingress_id == input.ingress_id
            && r.original_interaction_id == event
            && r.decision == decision
    });
    if !exact_replay
        && (headers["state"] != "executing"
            || headers["phase"] != "processing"
            || !headers["owner_kind"].is_null()
            || !headers["owner_id"].is_null())
    {
        return Err(invalid(
            "interaction has no current unowned execution custody",
        ));
    }
    Ok(Click {
        event,
        decision,
        identity: json!({"headers":headers,"payload":payload}),
    })
}

fn receipt_in(db: &Connection, id: &str) -> Result<Option<DecisionReceipt>> {
    let row: Option<(String, String, i64)> = db
        .query_row(
            "SELECT d.decision,d.ingress_id,d.interaction_id
         FROM cdr_recovery_publication_decisions d JOIN cdr_recovery_publication_proposals p
         ON p.id=d.proposal_id AND p.revision=d.revision
         WHERE d.proposal_id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(decision, ingress, event)| {
        Ok(DecisionReceipt {
            decision: match decision.as_str() {
                "approve_exact" => Decision::ApproveExact,
                "keep_held" => Decision::KeepHeld,
                _ => return Err(invalid("unsupported stored publication decision")),
            },
            original_ingress_id: ingress,
            original_interaction_id: event,
            already_recorded: true,
        })
    })
    .transpose()
}

fn existing_receipt(
    db: &Connection,
    input: &ConsentInput<'_>,
    stored: &StoredProposal,
    click: &Click,
) -> Result<Option<DecisionReceipt>> {
    let Some(receipt) = receipt_in(db, input.proposal_id)? else {
        return Ok(None);
    };
    if receipt.decision != click.decision {
        return Err(invalid(
            "decision already recorded; no conflicting overwrite",
        ));
    }
    if receipt.original_ingress_id != input.ingress_id
        || receipt.original_interaction_id != click.event
    {
        verify_fresh_in(db, stored, input.now)?;
    }
    Ok(Some(receipt))
}

/// The existing authenticated dispatcher owns the ingress. This storage API
/// rechecks that durable identity, not user-supplied actor/message fields.
pub fn record_consent(path: &Path, input: &ConsentInput<'_>) -> Result<DecisionReceipt> {
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_in(&tx, input.proposal_id)?;
    if stored.proposal.revision != input.revision {
        return Err(invalid("component revision differs from stored proposal"));
    }
    let click = click_in(&tx, input, &stored.proposal)?;
    if let Some(receipt) = existing_receipt(&tx, input, &stored, &click)? {
        tx.commit()?;
        return Ok(receipt);
    }
    verify_fresh_in(&tx, &stored, input.now)?;
    tx.execute(
        "INSERT INTO cdr_recovery_publication_decisions
         (proposal_id,revision,ingress_id,interaction_id,decision,recorded_at_bits)
         VALUES(?,?,?,?,?,?)",
        params![
            input.proposal_id,
            input.revision,
            input.ingress_id,
            click.event,
            click.decision.stored(),
            input.now.to_bits().to_string()
        ],
    )?;
    let receipt = receipt_in(&tx, input.proposal_id)?
        .ok_or_else(|| invalid("decision insert was ignored"))?;
    let exact_record: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_recovery_publication_decisions
         WHERE proposal_id=? AND revision=? AND recorded_at_bits=?)",
        params![
            input.proposal_id,
            input.revision,
            input.now.to_bits().to_string()
        ],
        |r| r.get(0),
    )?;
    if !exact_record
        || receipt.original_ingress_id != input.ingress_id
        || receipt.original_interaction_id != click.event
        || receipt.decision != click.decision
    {
        return Err(invalid("decision insert was altered"));
    }
    verify_fresh_in(&tx, &stored, input.now)?;
    let retained_click = click_in(&tx, input, &stored.proposal)?;
    if retained_click.identity != click.identity {
        return Err(invalid("interaction changed during decision commit"));
    }
    tx.commit()?;
    Ok(DecisionReceipt {
        already_recorded: false,
        ..receipt
    })
}

use super::{
    api::{self, Decision, DecisionInput, DecisionReceipt, StoredProposal},
    identity, invalid, proposal, snapshot,
};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::Path;

fn original_message(stored: &StoredProposal) -> Result<i64> {
    let row = &stored.snapshot["job"];
    let columns = row["columns"]
        .as_array()
        .ok_or_else(|| invalid("original row columns missing"))?;
    let index = columns
        .iter()
        .position(|v| v == "discord_message_id")
        .ok_or_else(|| invalid("original event column missing"))?;
    let cell = &row["rows"][0][index];
    if cell[0] != "integer" {
        return Err(invalid("original event identity missing"));
    }
    cell[1]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("original event identity invalid"))
}

fn cancellation_in(db: &Connection, job: &str) -> Result<Option<Value>> {
    Ok(db.query_row(
        "SELECT job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at
         FROM codex_request_cancellations WHERE job_id=?", [job],
        |r|Ok(json!([r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,
            r.get::<_,Option<i64>>(3)?,r.get::<_,Option<i64>>(4)?,r.get::<_,f64>(5)?.to_bits()])),
    ).optional()?)
}

fn require_applied_in(
    db: &Connection,
    stored: &StoredProposal,
    receipt: &DecisionReceipt,
) -> Result<()> {
    if receipt.decision != Decision::AbandonOnly {
        return Ok(());
    }
    let p = &stored.proposal;
    let queued: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_turn_queue WHERE job_id=?)",
        [&p.job_id],
        |r| r.get(0),
    )?;
    let expected = json!([
        p.job_id,
        p.thread_id,
        p.channel_id,
        p.owner_user_id,
        original_message(stored)?,
        receipt.recorded_at_bits
    ]);
    if queued || cancellation_in(db, &p.job_id)? != Some(expected) {
        return Err(invalid("disposition has no exact non-executable tombstone"));
    }
    Ok(())
}

pub(super) fn receipt_in(
    db: &Connection,
    stored: &StoredProposal,
) -> Result<Option<DecisionReceipt>> {
    let p = &stored.proposal;
    let row = db
        .query_row(
            "SELECT revision,ingress_id,interaction_id,decision,recorded_at_bits
         FROM cdr_recovery_abandonment_decisions WHERE proposal_id=?",
            [&p.id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((revision, ingress, event, choice, bits)) = row else {
        return Ok(None);
    };
    let recorded_at_bits: u64 = bits
        .parse()
        .map_err(|_| invalid("invalid disposition timestamp"))?;
    let now = f64::from_bits(recorded_at_bits);
    let decision = match choice.as_str() {
        "abandon_only" => Decision::AbandonOnly,
        "keep_held" => Decision::KeepHeld,
        _ => return Err(invalid("unsupported disposition")),
    };
    if revision != p.revision
        || event <= 0
        || ingress.is_empty()
        || !now.is_finite()
        || now < f64::from_bits(p.created_at_bits)
        || now >= f64::from_bits(p.expires_at_bits)
    {
        return Err(invalid("disposition identity or timestamp differs"));
    }
    let receipt = DecisionReceipt {
        proposal_id: p.id.clone(),
        revision,
        job_id: p.job_id.clone(),
        thread_id: p.thread_id.clone(),
        ingress_id: ingress,
        interaction_id: event,
        decision,
        recorded_at_bits,
    };
    require_applied_in(db, stored, &receipt)?;
    Ok(Some(receipt))
}

/// Atomically dispose one saved request. The runtime caller must retain its
/// `AdmissionGate` permit and shared target lock for this entire synchronous call.
/// This never releases the target, issues RPCs or replays original work.
pub fn record_decision(path: &Path, input: &DecisionInput<'_>) -> Result<DecisionReceipt> {
    if input.revision < 1 || !input.now.is_finite() || input.now < 0.0 {
        return Err(invalid("invalid decision revision or host time"));
    }
    let mut db = api::open(path, false)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = proposal::read_in(&tx, input.proposal_id)?;
    api::require_database(path, &stored)?;
    let p = &stored.proposal;
    if p.revision != input.revision {
        return Err(invalid("decision revision differs"));
    }
    if let Some(receipt) = receipt_in(&tx, &stored)? {
        let event = identity::click_in(&tx, &stored, input.ingress_id, input.decision, false)?;
        if receipt.ingress_id != input.ingress_id
            || receipt.interaction_id != event
            || receipt.decision != input.decision
        {
            return Err(invalid(
                "decision was already consumed by another interaction",
            ));
        }
        tx.commit()?;
        return Ok(receipt);
    }
    proposal::verify_fresh_in(&tx, path, &stored, input.now)?;
    let event = identity::click_in(&tx, &stored, input.ingress_id, input.decision, true)?;
    let receipt = DecisionReceipt {
        proposal_id: p.id.clone(),
        revision: p.revision,
        job_id: p.job_id.clone(),
        thread_id: p.thread_id.clone(),
        ingress_id: input.ingress_id.into(),
        interaction_id: event,
        decision: input.decision,
        recorded_at_bits: input.now.to_bits(),
    };
    if tx.execute(
        "INSERT INTO cdr_recovery_abandonment_decisions VALUES(?,?,?,?,?,?)",
        params![
            p.id,
            p.revision,
            input.ingress_id,
            event,
            api::decision_text(input.decision),
            receipt.recorded_at_bits.to_string()
        ],
    )? != 1
    {
        return Err(invalid("decision insert was ignored"));
    }
    if input.decision == Decision::AbandonOnly {
        abandon_in(&tx, path, &stored, &receipt)?;
    } else {
        proposal::verify_fresh_in(&tx, path, &stored, input.now)?;
    }
    let retained = proposal::read_in(&tx, &p.id)?;
    if serde_json::to_string(&retained)? != serde_json::to_string(&stored)?
        || receipt_in(&tx, &retained)? != Some(receipt.clone())
        || identity::click_in(&tx, &retained, input.ingress_id, input.decision, true)? != event
    {
        return Err(invalid("decision or original evidence was lost or altered"));
    }
    if proposal::latest_in(&tx, &p.job_id)? != p.revision {
        return Err(invalid("proposal was superseded during disposition"));
    }
    tx.commit()?;
    Ok(receipt)
}

fn abandon_in(
    db: &Connection,
    path: &Path,
    stored: &StoredProposal,
    receipt: &DecisionReceipt,
) -> Result<()> {
    let p = &stored.proposal;
    let now = f64::from_bits(receipt.recorded_at_bits);
    if db.execute(
        "INSERT INTO codex_request_cancellations
         (job_id,target_thread_id,channel_id,owner_user_id,discord_message_id,cancelled_at) VALUES(?,?,?,?,?,?)",
        params![p.job_id,p.thread_id,p.channel_id,p.owner_user_id,original_message(stored)?,now],
    )? != 1 { return Err(invalid("irreversible cancellation insert was ignored")); }
    let target = snapshot::Target {
        job: p.job_id.clone(),
        thread: p.thread_id.clone(),
        owner: p.owner_user_id,
        channel: p.channel_id,
    };
    let mut budget = 0;
    if snapshot::job_row_in(db, &p.job_id, &mut budget)? != stored.snapshot["job"]
        || snapshot::context_in(
            db,
            path,
            &target,
            &stored.source_ingress,
            false,
            &mut budget,
        )? != stored.snapshot["context"]
    {
        return Err(invalid("original evidence changed during disposition"));
    }
    if db.execute("DELETE FROM codex_turn_queue WHERE job_id=? AND target_thread_id=? AND state='pending' AND turn_id IS NULL",
        params![p.job_id,p.thread_id])? != 1
    { return Err(invalid("exact original Pending removal did not apply")); }
    require_applied_in(db, stored, receipt)?;
    let mut budget = 0;
    if snapshot::context_in(
        db,
        path,
        &target,
        &stored.source_ingress,
        false,
        &mut budget,
    )? != stored.snapshot["context"]
    {
        return Err(invalid(
            "disposition altered another request or a lifecycle barrier",
        ));
    }
    Ok(())
}

/// Durable read-only status. It does not manufacture a retry or release grant.
pub fn decision_status(path: &Path, id: &str, revision: i64) -> Result<Option<DecisionReceipt>> {
    let mut db = api::open(path, true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let stored = proposal::read_in(&tx, id)?;
    api::require_database(path, &stored)?;
    if revision < 1 || stored.proposal.revision != revision {
        return Err(invalid("status revision differs"));
    }
    let receipt = receipt_in(&tx, &stored)?;
    tx.commit()?;
    Ok(receipt)
}

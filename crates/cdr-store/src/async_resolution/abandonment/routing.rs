//! Read-only routing checks. Neither helper is consent or an execution permit.
use super::{
    api::{self, Decision, DeliveredProposal},
    binding, decision, identity, invalid, proposal,
};
use crate::Result;
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;

pub struct DecisionRouteInput<'a> {
    pub proposal_id: &'a str,
    pub revision: i64,
    pub interaction_id: i64,
    pub application_id: i64,
    pub channel_id: i64,
    pub owner_user_id: i64,
    pub source_message_id: i64,
    pub decision: Decision,
    pub now: f64,
}

fn require_mapping(db: &Connection, target: &str, channel: i64) -> Result<()> {
    let mapped: bool = db.query_row(
        "SELECT count(*)=1 AND MAX(codex_thread_id=?1 AND discord_thread_id=?2)
         FROM mirror_threads WHERE codex_thread_id=?1 OR discord_thread_id=?2",
        params![target, channel],
        |r| r.get(0),
    )?;
    if !mapped {
        return Err(invalid("exact original mapping is unavailable"));
    }
    Ok(())
}

/// Resolve only the original owner's held job in its original Discord channel.
/// No selected-target fallback, initialization or unbounded queue scan is used.
pub fn command_target(path: &Path, job_id: &str, channel: i64, owner: i64) -> Result<String> {
    if channel <= 0
        || owner <= 0
        || uuid::Uuid::parse_str(job_id).map_or(true, |id| id.to_string() != job_id)
    {
        return Err(invalid("exact job and authenticated actor are required"));
    }
    let mut db = api::open(path, true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    super::check_compatibility_in(&tx, super::FORMAT_VERSION)?;
    let target: String = tx.query_row(
        "SELECT target_thread_id FROM codex_turn_queue WHERE job_id=? AND channel_id=?
         AND owner_user_id=? AND state='pending' AND turn_id IS NULL AND goal_waiting=0
         AND discord_message_id>0 AND app_server_generation>0
         AND length(target_thread_id) BETWEEN 1 AND 256",
        params![job_id, channel, owner],
        |r| r.get(0),
    )?;
    if !crate::async_resolution::held_in(&tx, &target)? {
        return Err(invalid("original request is not held"));
    }
    require_mapping(&tx, &target, channel)?;
    tx.commit()?;
    Ok(target)
}

/// Authenticate the displayed decision before ACK and again under the target
/// lock. Only the same already-committed event can use historical receipt data.
pub fn authorize_decision(
    path: &Path,
    input: &DecisionRouteInput<'_>,
) -> Result<DeliveredProposal> {
    if input.interaction_id <= 0 || !input.now.is_finite() || input.now < 0.0 {
        return Err(invalid("invalid authenticated interaction identity"));
    }
    let mut db = api::open(path, true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let stored = proposal::read_in(&tx, input.proposal_id)?;
    api::require_database(path, &stored)?;
    let delivered = binding::read_in(&tx, input.proposal_id, input.revision)?;
    delivered.require_actor(
        input.application_id,
        input.channel_id,
        input.owner_user_id,
        input.source_message_id,
    )?;
    require_mapping(&tx, &delivered.proposal.thread_id, input.channel_id)?;
    if let Some(receipt) = decision::receipt_in(&tx, &stored)? {
        let ingress = format!("interaction:{}", input.interaction_id);
        if receipt.ingress_id != ingress
            || receipt.interaction_id != input.interaction_id
            || receipt.decision != input.decision
            || identity::click_in(&tx, &stored, &ingress, input.decision, false)?
                != input.interaction_id
        {
            return Err(invalid(
                "decision was already consumed by another interaction",
            ));
        }
    } else {
        proposal::verify_fresh_in(&tx, path, &stored, input.now)?;
    }
    tx.commit()?;
    Ok(delivered)
}

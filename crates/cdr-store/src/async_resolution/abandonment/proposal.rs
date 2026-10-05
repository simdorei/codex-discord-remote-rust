use super::{
    FORMAT_VERSION,
    api::{self, Proposal, ProposalInput, StoredProposal},
    invalid, snapshot,
};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::json;
use std::path::Path;

fn validate(input: &ProposalInput<'_>) -> Result<()> {
    if !api::valid_id(input.proposal_id)
        || uuid::Uuid::parse_str(input.job_id).is_err()
        || input.application_id <= 0
        || input.ingress_id.is_empty()
        || input.ingress_id.len() > 256
        || !input.now.is_finite()
        || !input.expires_at.is_finite()
        || input.now < 0.0
        || input.expires_at <= input.now
        || input.expires_at - input.now > 600.0
    {
        return Err(invalid("invalid proposal identity or bounded lifetime"));
    }
    Ok(())
}

/// The producer must hold the runtime admission permit and the same target lock
/// used by queue/control work. This API validates actual persisted ingress too.
pub fn propose(path: &Path, input: &ProposalInput<'_>) -> Result<Proposal> {
    validate(input)?;
    let mut db = api::open(path, false)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    super::check_compatibility_in(&tx, FORMAT_VERSION)?;
    let captured = snapshot::capture_in(&tx, path, input.job_id, input.ingress_id, true)?;
    let existing = read_optional_in(&tx, input.proposal_id)?;
    let revision = match &existing {
        Some(stored) => stored.proposal.revision,
        None => latest_in(&tx, input.job_id)?
            .checked_add(1)
            .ok_or_else(|| invalid("proposal revision exhausted"))?,
    };
    let review_text = format!(
        "Abandon saved request only?\nRequest: {}\nThread: {}\nProposal revision: {revision}\n\n\
         This one saved request will be permanently cancelled and never replayed.\n\
         The thread remains held; this does not enable new requests, stop the original execution, \
         withdraw Stop/Archive, unarchive a thread, or cancel published posts or schedules.\n\
         Choose Abandon saved request only or Keep held.\nExpires (host epoch seconds): {}",
        input.job_id, captured.target.thread, input.expires_at,
    );
    let proposal = Proposal {
        id: input.proposal_id.into(),
        revision,
        job_id: input.job_id.into(),
        thread_id: captured.target.thread,
        owner_user_id: captured.target.owner,
        channel_id: captured.target.channel,
        application_id: input.application_id,
        created_at_bits: input.now.to_bits(),
        expires_at_bits: input.expires_at.to_bits(),
        review_sha256: api::digest(&review_text),
        review_text,
    };
    let stored = StoredProposal {
        version: FORMAT_VERSION,
        proposal: proposal.clone(),
        source_ingress: input.ingress_id.into(),
        snapshot: captured.evidence,
    };
    let encoded = serde_json::to_string(&stored)?;
    if encoded.len() > api::MAX_SEAL_BYTES {
        return Err(invalid("private proposal seal exceeds bound"));
    }
    if let Some(previous) = existing {
        if serde_json::to_string(&previous)? != encoded {
            return Err(invalid("proposal identity was reused"));
        }
    } else {
        let changed = tx.execute(
            "INSERT INTO cdr_recovery_abandonment_proposals
             (id,format_version,revision,job_id,target_thread_id,owner_user_id,channel_id,application_id,seal_json,seal_sha256)
             VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![proposal.id,FORMAT_VERSION,revision,proposal.job_id,proposal.thread_id,
                proposal.owner_user_id,proposal.channel_id,proposal.application_id,encoded,api::digest(&encoded)],
        )?;
        if changed != 1 {
            return Err(invalid("proposal was not stored"));
        }
    }
    let retained = read_in(&tx, &proposal.id)?;
    if serde_json::to_string(&retained)? != encoded {
        return Err(invalid("proposal was altered"));
    }
    verify_fresh_in(&tx, path, &retained, input.now)?;
    tx.commit()?;
    Ok(proposal)
}

pub(super) fn latest_in(db: &Connection, job: &str) -> Result<i64> {
    Ok(db
        .query_row(
            "SELECT MAX(revision) FROM cdr_recovery_abandonment_proposals WHERE job_id=?",
            [job],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(0))
}

fn read_optional_in(db: &Connection, id: &str) -> Result<Option<StoredProposal>> {
    if !api::valid_id(id) {
        return Err(invalid("invalid proposal identity"));
    }
    super::check_compatibility_in(db, FORMAT_VERSION)?;
    let row = db.query_row(
        "SELECT seal_json,seal_sha256,format_version,revision,job_id,target_thread_id,owner_user_id,channel_id,application_id
         FROM cdr_recovery_abandonment_proposals WHERE id=? AND length(CAST(seal_json AS BLOB))<=?",
        params![id,i64::try_from(api::MAX_SEAL_BYTES).map_err(|_|invalid("seal size overflow"))?], |r|Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,
            json!([r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,
                r.get::<_,i64>(6)?,r.get::<_,i64>(7)?,r.get::<_,i64>(8)?]),
        )),
    ).optional()?;
    let Some((encoded, hash, columns)) = row else {
        return Ok(None);
    };
    let stored: StoredProposal = serde_json::from_str(&encoded)?;
    let p = &stored.proposal;
    let created = f64::from_bits(p.created_at_bits);
    let expiry = f64::from_bits(p.expires_at_bits);
    if api::digest(&encoded) != hash
        || stored.version != FORMAT_VERSION
        || p.id != id
        || p.revision < 1
        || p.owner_user_id <= 0
        || p.channel_id <= 0
        || p.application_id <= 0
        || columns
            != json!([
                stored.version,
                p.revision,
                p.job_id,
                p.thread_id,
                p.owner_user_id,
                p.channel_id,
                p.application_id
            ])
        || api::digest(&p.review_text) != p.review_sha256
        || !created.is_finite()
        || !expiry.is_finite()
        || created < 0.0
        || expiry <= created
        || expiry - created > 600.0
    {
        return Err(invalid(
            "stored identity differs from its immutable private seal",
        ));
    }
    Ok(Some(stored))
}

pub(super) fn read_in(db: &Connection, id: &str) -> Result<StoredProposal> {
    read_optional_in(db, id)?.ok_or_else(|| invalid("proposal is missing or oversized"))
}

pub(super) fn verify_fresh_in(
    db: &Connection,
    path: &Path,
    stored: &StoredProposal,
    now: f64,
) -> Result<()> {
    let p = &stored.proposal;
    api::require_database(path, stored)?;
    if !now.is_finite()
        || now < f64::from_bits(p.created_at_bits)
        || now >= f64::from_bits(p.expires_at_bits)
        || latest_in(db, &p.job_id)? != p.revision
        || snapshot::capture_in(db, path, &p.job_id, &stored.source_ingress, false)?.evidence
            != stored.snapshot
    {
        return Err(invalid(
            "proposal expired, superseded or its exact evidence changed",
        ));
    }
    Ok(())
}

/// Only a confirmed delivery of this exact body may bind a proposal. Unknown
/// HTTP outcomes must be reconciled by receipt identity, never by guessing.
pub fn bind_delivery(path: &Path, id: &str, message: i64, body_sha: &str, now: f64) -> Result<()> {
    if message <= 0 {
        return Err(invalid("invalid source message"));
    }
    let mut db = api::open(path, false)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_in(&tx, id)?;
    verify_fresh_in(&tx, path, &stored, now)?;
    let p = &stored.proposal;
    if p.review_sha256 != body_sha {
        return Err(invalid("delivered body differs"));
    }
    let existing: Option<(i64,i64,String)> = tx.query_row(
        "SELECT revision,message_id,body_sha256 FROM cdr_recovery_abandonment_deliveries WHERE proposal_id=?",
        [id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional()?;
    let expected = (p.revision, message, body_sha.to_owned());
    if let Some(existing) = existing {
        if existing != expected {
            return Err(invalid("delivery is already bound elsewhere"));
        }
    } else if tx.execute(
        "INSERT INTO cdr_recovery_abandonment_deliveries VALUES(?,?,?,?)",
        params![id, p.revision, message, body_sha],
    )? != 1
    {
        return Err(invalid("delivery binding was ignored"));
    }
    let retained = super::binding::read_in(&tx, id, p.revision)?;
    if retained.message_id != message || retained.proposal != *p {
        return Err(invalid("delivery binding was altered"));
    }
    verify_fresh_in(&tx, path, &stored, now)?;
    tx.commit()?;
    Ok(())
}

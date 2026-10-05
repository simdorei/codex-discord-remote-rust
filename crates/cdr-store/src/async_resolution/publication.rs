//! Durable publication intent, not an execution or publisher-exclusion grant.
//!
//! The producer must separately verify terminal/control disposition, present the
//! exact proposal, and retain a real publisher guard. This ledger alone cannot
//! consume a Pending job, issue an RPC, release a hold, or certify those facts.
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Result, StoreError};

mod binding;
mod consent;
mod schema;
mod snapshot;

pub use binding::{DeliveredProposal, delivered_proposal};
pub use consent::{ConsentInput, Decision, DecisionReceipt, record_consent};
pub use schema::check_compatibility_in;
pub(crate) use schema::{migrate_schema, schema_current};

pub const COMPONENT: &str = "recovery_publication_consent";
pub const FORMAT_VERSION: i64 = 1;
const MAX_SEAL_BYTES: usize = 524_288;
const MAX_REVIEW_BYTES: usize = 8_000;
const MAX_CONTEXT_BYTES: usize = 65_536;
const MAX_LIFETIME_SECONDS: f64 = 600.0;

pub struct ProposalInput<'a> {
    pub proposal_id: &'a str,
    pub job_id: &'a str,
    pub application_id: i64,
    pub review_text: &'a str,
    /// Descriptive review material only. Hashes and booleans are not live guards.
    pub review_context: &'a Value,
    pub now: f64,
    pub expires_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub id: String,
    pub revision: i64,
    pub job_id: String,
    pub thread_id: String,
    pub owner_user_id: i64,
    pub channel_id: i64,
    pub application_id: i64,
    pub created_at_bits: u64,
    pub expires_at_bits: u64,
    pub review_text: String,
    pub review_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProposal {
    version: i64,
    proposal: Proposal,
    snapshot: Value,
    review_context: Value,
}

fn invalid(reason: &str) -> StoreError {
    StoreError::Integrity(format!("recovery publication consent held: {reason}"))
}

fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn validate_input(input: &ProposalInput<'_>) -> Result<()> {
    if !valid_id(input.proposal_id)
        || input.application_id <= 0
        || input.job_id.trim().is_empty()
        || input.job_id.len() > 128
        || input.review_text.trim().is_empty()
        || input.review_text.len() > MAX_REVIEW_BYTES
        || !input.review_context.is_object()
        || serde_json::to_vec(input.review_context)?.len() > MAX_CONTEXT_BYTES
        || !input.now.is_finite()
        || !input.expires_at.is_finite()
        || input.now < 0.0
        || input.expires_at <= input.now
        || input.expires_at - input.now > MAX_LIFETIME_SECONDS
    {
        return Err(invalid(
            "invalid or unbounded proposal identity, review or lifetime",
        ));
    }
    Ok(())
}

/// Create an immutable review intent. Repeating the same producer identity must
/// match every original byte; a new revision supersedes, but never deletes, old
/// decisions. Neither result is a dispatch permission.
pub fn propose(path: &Path, input: &ProposalInput<'_>) -> Result<Proposal> {
    validate_input(input)?;
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    check_compatibility_in(&tx, FORMAT_VERSION)?;
    let captured = snapshot::capture_in(&tx, input.job_id)?;
    let existing = read_optional_in(&tx, input.proposal_id)?;
    let revision = if let Some(existing) = &existing {
        existing.proposal.revision
    } else {
        latest_revision_in(&tx, input.job_id)?
            .checked_add(1)
            .ok_or_else(|| invalid("proposal revision exhausted"))?
    };
    let proposal = Proposal {
        id: input.proposal_id.into(),
        revision,
        job_id: input.job_id.into(),
        thread_id: captured.thread,
        owner_user_id: captured.owner,
        channel_id: captured.channel,
        application_id: input.application_id,
        created_at_bits: input.now.to_bits(),
        expires_at_bits: input.expires_at.to_bits(),
        review_text: input.review_text.into(),
        review_sha256: digest(input.review_text),
    };
    let stored = StoredProposal {
        version: FORMAT_VERSION,
        proposal: proposal.clone(),
        snapshot: captured.seal,
        review_context: input.review_context.clone(),
    };
    let encoded = serde_json::to_string(&stored)?;
    if encoded.len() > MAX_SEAL_BYTES {
        return Err(invalid("proposal seal exceeds bound"));
    }
    if let Some(existing) = existing {
        if serde_json::to_string(&existing)? != encoded {
            return Err(invalid("producer identity reused with different proposal"));
        }
    } else {
        tx.execute(
            "INSERT INTO cdr_recovery_publication_proposals
             (id,format_version,revision,job_id,target_thread_id,owner_user_id,channel_id,
              application_id,seal_json,seal_sha256) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![
                proposal.id,
                FORMAT_VERSION,
                revision,
                proposal.job_id,
                proposal.thread_id,
                proposal.owner_user_id,
                proposal.channel_id,
                proposal.application_id,
                encoded,
                digest(&encoded)
            ],
        )?;
    }
    let retained = read_in(&tx, &proposal.id)?;
    if serde_json::to_string(&retained)? != encoded {
        return Err(invalid("proposal insert was lost or altered"));
    }
    verify_fresh_in(&tx, &retained, input.now)?;
    tx.commit()?;
    Ok(proposal)
}

fn latest_revision_in(db: &Connection, job: &str) -> Result<i64> {
    Ok(db
        .query_row(
            "SELECT MAX(revision) FROM cdr_recovery_publication_proposals WHERE job_id=?",
            [job],
            |r| r.get::<_, Option<i64>>(0),
        )?
        .unwrap_or(0))
}

fn read_optional_in(db: &Connection, id: &str) -> Result<Option<StoredProposal>> {
    check_compatibility_in(db, FORMAT_VERSION)?;
    let row = db
        .query_row(
            "SELECT seal_json,seal_sha256,format_version,revision,job_id,target_thread_id,
         owner_user_id,channel_id,application_id FROM cdr_recovery_publication_proposals
         WHERE id=? AND length(CAST(seal_json AS BLOB))<=?",
            params![
                id,
                i64::try_from(MAX_SEAL_BYTES).map_err(|_| invalid("seal bound overflow"))?
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    serde_json::json!([
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                        r.get::<_, i64>(8)?
                    ]),
                ))
            },
        )
        .optional()?;
    let Some((encoded, sha, columns)) = row else {
        return Ok(None);
    };
    let stored: StoredProposal = serde_json::from_str(&encoded)?;
    let p = &stored.proposal;
    if digest(&encoded) != sha
        || stored.version != FORMAT_VERSION
        || p.id != id
        || columns
            != serde_json::json!([
                stored.version,
                p.revision,
                p.job_id,
                p.thread_id,
                p.owner_user_id,
                p.channel_id,
                p.application_id
            ])
        || p.review_sha256 != digest(&p.review_text)
    {
        return Err(invalid(
            "stored proposal identity differs from its immutable seal",
        ));
    }
    Ok(Some(stored))
}

fn read_in(db: &Connection, id: &str) -> Result<StoredProposal> {
    read_optional_in(db, id)?.ok_or_else(|| invalid("proposal is missing or oversized"))
}

fn verify_fresh_in(db: &Connection, stored: &StoredProposal, now: f64) -> Result<()> {
    check_compatibility_in(db, FORMAT_VERSION)?;
    let p = &stored.proposal;
    if !now.is_finite()
        || now < f64::from_bits(p.created_at_bits)
        || now >= f64::from_bits(p.expires_at_bits)
        || latest_revision_in(db, &p.job_id)? != p.revision
        || snapshot::capture_in(db, &p.job_id)?.seal != stored.snapshot
    {
        return Err(invalid(
            "proposal expired, superseded or its local evidence changed",
        ));
    }
    Ok(())
}

/// Called only after confirmed delivery of the exact review text. An unknown
/// HTTP outcome must not call this. Rebinding an existing message is forbidden.
pub fn bind_delivery(path: &Path, id: &str, message: i64, body_sha: &str, now: f64) -> Result<()> {
    if message <= 0 {
        return Err(invalid("invalid proposal message identity"));
    }
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = read_in(&tx, id)?;
    verify_fresh_in(&tx, &stored, now)?;
    let p = &stored.proposal;
    if p.review_sha256 != body_sha {
        return Err(invalid("delivered review text differs"));
    }
    tx.execute(
        "INSERT OR IGNORE INTO cdr_recovery_publication_deliveries
         (proposal_id,revision,message_id,body_sha256) VALUES(?,?,?,?)",
        params![id, p.revision, message, body_sha],
    )?;
    let retained: (i64, i64, String) = tx.query_row(
        "SELECT revision,message_id,body_sha256 FROM cdr_recovery_publication_deliveries
         WHERE proposal_id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if retained != (p.revision, message, body_sha.into()) {
        return Err(invalid(
            "delivery was lost, altered or already bound elsewhere",
        ));
    }
    verify_fresh_in(&tx, &stored, now)?;
    tx.commit()?;
    Ok(())
}

//! Public value types for the authenticated no-replay disposition path.
use crate::Result;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

pub struct ProposalInput<'a> {
    pub proposal_id: &'a str,
    pub job_id: &'a str,
    pub ingress_id: &'a str,
    pub application_id: i64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    AbandonOnly,
    KeepHeld,
}

pub struct DecisionInput<'a> {
    pub proposal_id: &'a str,
    pub revision: i64,
    pub ingress_id: &'a str,
    pub decision: Decision,
    pub now: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionReceipt {
    pub proposal_id: String,
    pub revision: i64,
    pub job_id: String,
    pub thread_id: String,
    pub ingress_id: String,
    pub interaction_id: i64,
    pub decision: Decision,
    pub recorded_at_bits: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeliveredProposal {
    pub proposal: Proposal,
    pub message_id: i64,
}

impl DeliveredProposal {
    pub fn require_actor(
        &self,
        application: i64,
        channel: i64,
        actor: i64,
        message: i64,
    ) -> Result<()> {
        let p = &self.proposal;
        if application <= 0
            || channel <= 0
            || actor <= 0
            || message <= 0
            || (application, channel, actor, message)
                != (
                    p.application_id,
                    p.channel_id,
                    p.owner_user_id,
                    self.message_id,
                )
        {
            return Err(super::invalid("authenticated delivery identity differs"));
        }
        Ok(())
    }
}

pub(super) const MAX_SEAL_BYTES: usize = 524_288;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredProposal {
    pub version: i64,
    pub proposal: Proposal,
    pub source_ingress: String,
    pub snapshot: Value,
}

pub(super) fn open(path: &Path, readonly: bool) -> Result<Connection> {
    let flags = if readonly {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let db = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(if readonly {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(5)
    })?;
    Ok(db)
}

pub(super) fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub(super) fn database_identity(path: &Path) -> Result<String> {
    std::fs::canonicalize(path)
        .map_err(|_| super::invalid("database identity is unavailable"))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| super::invalid("database identity is not UTF-8"))
}

pub(super) fn require_database(path: &Path, stored: &StoredProposal) -> Result<()> {
    if stored.snapshot["context"]["database"] != database_identity(path)? {
        return Err(super::invalid(
            "proposal belongs to another database installation",
        ));
    }
    Ok(())
}

pub(super) fn decision_text(value: Decision) -> &'static str {
    match value {
        Decision::AbandonOnly => "abandon_only",
        Decision::KeepHeld => "keep_held",
    }
}

//! Read-only delivery identity for authenticated component admission.
//! A historical binding may be read after expiry; it is never fresh consent.
use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::{Proposal, invalid, read_in, valid_id};
use crate::Result;

#[derive(Debug, PartialEq)]
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
        if application <= 0
            || channel <= 0
            || actor <= 0
            || message <= 0
            || self.proposal.application_id != application
            || self.proposal.channel_id != channel
            || self.proposal.owner_user_id != actor
            || self.message_id != message
        {
            return Err(invalid("authenticated delivery identity does not match"));
        }
        Ok(())
    }
}

/// Never initializes, migrates or repairs a database while routing a click.
pub fn delivered_proposal(path: &Path, id: &str, revision: i64) -> Result<DeliveredProposal> {
    if !valid_id(id) || revision <= 0 {
        return Err(invalid("invalid delivered proposal identity"));
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_millis(100))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let stored = read_in(&tx, id)?;
    let binding: (i64, i64, String) = tx.query_row(
        "SELECT revision,message_id,body_sha256 FROM cdr_recovery_publication_deliveries
         WHERE proposal_id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if stored.proposal.revision != revision
        || binding.0 != revision
        || binding.1 <= 0
        || binding.2 != stored.proposal.review_sha256
    {
        return Err(invalid("delivered proposal revision or body changed"));
    }
    tx.commit()?;
    Ok(DeliveredProposal {
        proposal: stored.proposal,
        message_id: binding.1,
    })
}

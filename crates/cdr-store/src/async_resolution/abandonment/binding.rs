use super::{
    api::{self, DeliveredProposal},
    invalid, proposal,
};
use crate::Result;
use rusqlite::{Connection, TransactionBehavior};
use std::path::Path;

pub(super) fn read_in(db: &Connection, id: &str, revision: i64) -> Result<DeliveredProposal> {
    let stored = proposal::read_in(db, id)?;
    let p = stored.proposal;
    let (actual,message,hash): (i64,i64,String) = db.query_row(
        "SELECT revision,message_id,body_sha256 FROM cdr_recovery_abandonment_deliveries WHERE proposal_id=?",
        [id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    )?;
    if revision < 1
        || p.revision != revision
        || actual != revision
        || message <= 0
        || hash != p.review_sha256
    {
        return Err(invalid("displayed proposal identity differs"));
    }
    Ok(DeliveredProposal {
        proposal: p,
        message_id: message,
    })
}

/// Historical read-only routing identity, not fresh consent or release authority.
pub fn delivered_proposal(path: &Path, id: &str, revision: i64) -> Result<DeliveredProposal> {
    let mut db = api::open(path, true)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let stored = proposal::read_in(&tx, id)?;
    api::require_database(path, &stored)?;
    let result = read_in(&tx, id, revision)?;
    tx.commit()?;
    Ok(result)
}

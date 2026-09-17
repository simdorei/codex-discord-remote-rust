//! Immutable preparation stamp for a separate, one-shot async reply admission.
use super::{Policy, get_connection};
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    policy: Option<Policy>,
    failure_id: Option<i64>,
}

pub fn capture(path: &Path, thread: &str) -> Result<Stamp> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
    capture_in(&tx, thread)
}

pub(crate) fn capture_in(db: &Connection, thread: &str) -> Result<Stamp> {
    let policy = get_connection(db, thread)?;
    let failure: Option<(i64, Option<String>)> = db.query_row(
        "SELECT usage_failure_id,usage_failure_state FROM codex_reserve_policy WHERE thread_id=?",
        [thread], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional()?;
    if policy
        .as_ref()
        .is_some_and(|p| !matches!(p.state.as_str(), "ordinary" | "reserve"))
        || failure
            .as_ref()
            .is_some_and(|(_, state)| state.as_deref() == Some("pending"))
    {
        return Err(StoreError::Integrity(
            "async reply held by unresolved Reserve preparation".into(),
        ));
    }
    Ok(Stamp {
        policy,
        failure_id: failure.map(|(id, _)| id),
    })
}

pub(crate) fn require_in(db: &Connection, thread: &str, expected: &Stamp) -> Result<()> {
    if &capture_in(db, thread)? != expected {
        return Err(StoreError::Integrity(
            "async reply Reserve preparation changed".into(),
        ));
    }
    Ok(())
}

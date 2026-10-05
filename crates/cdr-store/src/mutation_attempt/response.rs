//! One-way replies retain original custody; flush is not remote acknowledgement.
use crate::{Result, StoreError};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

mod authority;
mod schema;
mod wire;
pub use authority::capture;
pub(crate) use schema::{migrate_schema, schema_current};
pub(crate) use wire::record_terminal_in;
pub use wire::{begin, finish};

pub struct Scope<'a> {
    pub runtime: &'a str,
    pub resident: &'a str,
    pub generation: i64,
    pub request: &'a Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    key: String,
    runtime: String,
    resident: String,
    generation: i64,
    request_sha256: String,
    thread: String,
    turn: String,
    job: String,
    // Keep the serialized job opaque: f64 timestamps must not round-trip via Value.
    original_job: String,
    mapping: Option<String>,
    stop_sequence: i64,
}

fn refused() -> StoreError {
    StoreError::Integrity(
        "original response custody changed or is held; no response or replay".into(),
    )
}

fn digest(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

pub fn check(path: &Path, thread: &str) -> Result<()> {
    require_unheld_in(&super::existing(path)?, thread)
}

pub fn check_all(path: &Path) -> Result<()> {
    require_all_resolved_in(&super::existing(path)?)
}

pub fn require_all_resolved_in(db: &Connection) -> Result<()> {
    let held: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_server_responses
        WHERE phase='admitted')",
        [],
        |row| row.get(0),
    )?;
    if held {
        return Err(refused());
    }
    Ok(())
}

pub fn require_unheld_in(db: &Connection, thread: &str) -> Result<()> {
    unheld_except(db, thread, "")
}

fn unheld_except(db: &Connection, thread: &str, own: &str) -> Result<()> {
    let held: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_server_responses
        WHERE target_thread_id=? AND phase='admitted' AND request_key<>?)",
        rusqlite::params![thread, own],
        |row| row.get(0),
    )?;
    if held {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

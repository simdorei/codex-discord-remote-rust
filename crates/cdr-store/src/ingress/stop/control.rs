//! Durable Running-stop receipt and one-use original interrupt authority.
use crate::{Result, StoreError};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

mod admission;
mod dispatch;
mod late_start;
mod schema;
mod terminal;
pub use admission::accept_running;
pub use dispatch::{begin_wire, claim, finish_wire, record_error, validate_claim_in};
pub(crate) use late_start::bind_in as bind_late_start_in;
pub(crate) use schema::{migrate_schema, schema_current};
pub(crate) use terminal::record_terminal_in;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StopControl {
    pub operation_id: String,
    pub target: String,
    pub channel: i64,
    pub owner: i64,
    pub resident: String,
    pub generation: i64,
    pub turn: String,
    pub binding: Value,
    // JSON records remain opaque strings across DB reads: parsing a timestamp
    // through Value must not alter the immutable original custody evidence.
    pub jobs: Vec<String>,
    pub can_settle: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StopClaim {
    pub control: StopControl,
    pub token: String,
}

fn refused() -> StoreError {
    StoreError::Integrity("original stop control authority differs; no interrupt or replay".into())
}

fn existing(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.busy_timeout(std::time::Duration::from_millis(500))?;
    Ok(db)
}

pub fn pending_after(path: &Path, after: i64) -> Result<Vec<(i64, StopControl)>> {
    let db = existing(path)?;
    let rows = db
        .prepare(
            "SELECT sequence,record_json FROM cdr_stop_controls
        WHERE phase='accepted' AND sequence>? ORDER BY sequence LIMIT 16",
        )?
        .query_map([after], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(seq, value)| Ok((seq, serde_json::from_str(&value)?)))
        .collect()
}

pub fn target_is_held(path: &Path, target: &str) -> Result<bool> {
    held_in(&existing(path)?, target)
}

fn held_in(db: &Connection, target: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_stop_controls
        WHERE target_thread_id=? AND phase<>'settled')",
        [target],
        |row| row.get(0),
    )?)
}

pub fn require_unheld_in(db: &Connection, target: &str) -> Result<()> {
    if held_in(db, target)? {
        return Err(refused());
    }
    Ok(())
}

fn retained_in(db: &Connection, control: &StopControl) -> Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM cdr_stop_controls WHERE operation_id=?
        AND target_thread_id=? AND resident_owner=? AND generation=? AND turn_id=? AND record_json=?)",
        rusqlite::params![control.operation_id,control.target,control.resident,control.generation,
            control.turn,serde_json::to_string(control)?], |row| row.get(0))?)
}

/// Status is evidence only, never permission to replay a request.
pub fn phase(path: &Path, operation: &str) -> Result<Option<String>> {
    Ok(existing(path)?
        .query_row(
            "SELECT phase FROM cdr_stop_controls WHERE operation_id=?",
            [operation],
            |row| row.get(0),
        )
        .optional()?)
}

//! Durable identity of a /new first turn, independent of the running queue/outbox.
mod claim;
mod intent;
mod notice;
mod schema;
mod verification;

pub use claim::{DeliveryGuard, acknowledgement_key, output_hold};
pub(crate) use claim::{confirm_receipt_in, validate_claim_in};
pub(crate) use intent::seed_in;
pub(crate) use intent::{bind_running_in, promote_in};
pub use intent::{get, get_by_ingress};
pub use notice::{acknowledgement_sendable, release_acknowledgement};
pub(crate) use notice::{notice_claimed_in, notice_rejected_in};
pub(crate) use schema::{migrate_schema, schema_current};
pub use verification::{CheckpointUpdate, checkpoint, pending, warning_text};

use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub struct NewReplySeed<'a> {
    pub state_db: &'a Path,
    pub acknowledgement: &'a str,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    pub ingress_id: String,
    pub job_id: String,
    pub thread_id: String,
    pub cwd: String,
    pub state_db: String,
    pub channel_id: i64,
    pub origin_channel_id: i64,
    pub event_id: Option<i64>,
    pub kind: crate::ingress::IngressKind,
    pub creation_generation: i64,
    pub prompt_sha256: String,
    pub acknowledgement: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewReply {
    pub identity: Identity,
    pub turn_id: Option<String>,
    pub accepted_at: Option<f64>,
    pub state: String,
    pub version: i64,
    pub scan: serde_json::Value,
    pub last_error: String,
    pub confirmation_delivered: bool,
    pub warning_due: i64,
    pub acknowledgement_recovery_allowed: bool,
}

pub(crate) fn get_in(connection: &Connection, job: &str) -> Result<Option<NewReply>> {
    let row = connection.query_row(
        "SELECT identity_json,turn_id,accepted_at,state,version,scan_json,last_error,
         confirmation_delivered,warning_due,ack_recovery_allowed FROM codex_new_first_replies WHERE job_id=?",
        [job], |row| Ok((row.get::<_,String>(0)?,row.get(1)?,row.get(2)?,
            row.get(3)?,row.get(4)?,row.get::<_,String>(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?)),
    ).optional()?;
    row.map(
        |(
            identity,
            turn_id,
            accepted_at,
            state,
            version,
            scan,
            last_error,
            confirmation_delivered,
            warning_due,
            acknowledgement_recovery_allowed,
        )| {
            Ok(NewReply {
                identity: serde_json::from_str(&identity)?,
                turn_id,
                accepted_at,
                state,
                version,
                scan: serde_json::from_str(&scan)?,
                last_error,
                confirmation_delivered,
                warning_due,
                acknowledgement_recovery_allowed,
            })
        },
    )
    .transpose()
}

pub fn validate_current(path: &Path, job: &str) -> Result<()> {
    let connection = open_initialized(path)?;
    if let Some(record) = get_in(&connection, job)? {
        claim::validate_identity_in(&connection, &record)?;
    }
    Ok(())
}

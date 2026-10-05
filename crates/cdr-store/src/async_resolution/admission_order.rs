//! Durable first-admission order, not permission to execute or release a hold.
//!
//! Legacy receipts never become fresh merely because this schema was installed.
//! Only the original admission transaction may record a new identity digest.
use rusqlite::{Connection, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Result, StoreError, ingress::NewIngress};

mod schema;
pub use schema::check_compatibility_in;
pub(crate) use schema::{migrate_schema, schema_current};

pub const COMPONENT: &str = "recovery_admission_order";
pub const FORMAT_VERSION: i64 = 1;

fn invalid(reason: &str) -> StoreError {
    StoreError::Integrity(format!("recovery admission order held: {reason}"))
}

// The schema cache validates structure only. Mutable capability requirements
// must be checked in the same transaction as each new durable admission.
fn require_current_format(db: &Connection) -> Result<()> {
    let required: i64 = db.query_row(
        "SELECT format_version FROM cdr_runtime_capability_requirements WHERE component=?",
        [COMPONENT],
        |row| row.get(0),
    )?;
    if required != FORMAT_VERSION {
        return Err(invalid("unsupported current admission order format"));
    }
    Ok(())
}

#[derive(PartialEq, Serialize)]
struct Identity {
    version: i64,
    kind: String,
    event_id: Option<i64>,
    application_id: Option<i64>,
    channel_id: i64,
    owner_user_id: i64,
    source_message_id: Option<i64>,
    payload_json: String,
    runtime_id: Option<String>,
    target_thread_id: Option<String>,
    canonical_owner: Option<String>,
    created_at_bits: u64,
}

fn identity_in(db: &Connection, id: &str) -> Result<Identity> {
    Ok(db.query_row(
        "SELECT version,kind,event_id,application_id,channel_id,owner_user_id,
         source_message_id,payload_json,runtime_id,target_thread_id,canonical_owner,created_at
         FROM discord_ingress_journal WHERE ingress_id=?",
        [id],
        |row| {
            Ok(Identity {
                version: row.get(0)?,
                kind: row.get(1)?,
                event_id: row.get(2)?,
                application_id: row.get(3)?,
                channel_id: row.get(4)?,
                owner_user_id: row.get(5)?,
                source_message_id: row.get(6)?,
                payload_json: row.get(7)?,
                runtime_id: row.get(8)?,
                target_thread_id: row.get(9)?,
                canonical_owner: row.get(10)?,
                created_at_bits: row.get::<_, f64>(11)?.to_bits(),
            })
        },
    )?)
}

/// Private original evidence carried through all writes of this admission.
/// This is neither a persisted release nor permission to execute a request.
pub(crate) struct RecordedAdmission {
    ingress_id: String,
    identity: Identity,
    sequence: i64,
    digest: String,
}

impl RecordedAdmission {
    pub(crate) fn verify_in(&self, db: &Connection) -> Result<()> {
        if db.is_autocommit() {
            return Err(StoreError::ActiveTransaction);
        }
        require_current_format(db)?;
        let retained: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM cdr_recovery_ingress_order WHERE sequence=?1
             AND ingress_id=?2 AND kind=?3 AND event_id IS ?4 AND origin='admitted' AND identity_sha256=?5)",
            params![self.sequence,self.ingress_id,self.identity.kind,self.identity.event_id,self.digest],
            |row|row.get(0),
        )?;
        if !retained || identity_in(db, &self.ingress_id)? != self.identity {
            return Err(invalid("admission identity changed after ordinal INSERT"));
        }
        Ok(())
    }
}

/// Call only after the original journal INSERT, inside its IMMEDIATE transaction.
/// Duplicate/legacy branches must return before this hook. Failure rolls back
/// the journal, ordinal and processed-message receipt together.
pub(crate) fn record_new_in(
    db: &Connection,
    request: &NewIngress,
    runtime: Option<&str>,
) -> Result<RecordedAdmission> {
    if db.is_autocommit() {
        return Err(StoreError::ActiveTransaction);
    }
    require_current_format(db)?;
    let expected = Identity {
        version: 1,
        kind: request.kind.as_str().into(),
        event_id: request.event_id,
        application_id: request.application_id,
        channel_id: request.channel_id,
        owner_user_id: request.owner_user_id,
        source_message_id: request.source_message_id,
        payload_json: request.payload.to_string(),
        runtime_id: runtime.map(str::to_owned),
        target_thread_id: request.target_thread_id.clone(),
        canonical_owner: request.canonical_owner.clone(),
        // SQLite stores either sign of zero as the same REAL zero.
        created_at_bits: if request.now == 0.0 {
            0.0_f64.to_bits()
        } else {
            request.now.to_bits()
        },
    };
    if identity_in(db, &request.ingress_id)? != expected {
        return Err(invalid("original persisted ingress differs from admission"));
    }
    let previous: i64 = db.query_row(
        "SELECT COALESCE(MAX(sequence),0) FROM cdr_recovery_ingress_order",
        [],
        |row| row.get(0),
    )?;
    previous
        .checked_add(1)
        .ok_or_else(|| invalid("durable sequence exhausted"))?;
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&expected)?));
    if db.execute(
        "INSERT INTO cdr_recovery_ingress_order
         (ingress_id,kind,event_id,origin,identity_sha256) VALUES(?,?,?,'admitted',?)",
        params![
            request.ingress_id,
            request.kind.as_str(),
            request.event_id,
            digest
        ],
    )? != 1
    {
        return Err(invalid(
            "ordinal INSERT did not retain the original admission",
        ));
    }
    let sequence: i64 = db.query_row(
        "SELECT sequence FROM cdr_recovery_ingress_order WHERE sequence>?1
         AND ingress_id=?2 AND kind=?3 AND event_id IS ?4 AND origin='admitted' AND identity_sha256=?5",
        params![previous,request.ingress_id,request.kind.as_str(),request.event_id,digest],
        |row|row.get(0),
    )?;
    let recorded = RecordedAdmission {
        ingress_id: request.ingress_id.clone(),
        identity: expected,
        sequence,
        digest,
    };
    recorded.verify_in(db)?;
    Ok(recorded)
}

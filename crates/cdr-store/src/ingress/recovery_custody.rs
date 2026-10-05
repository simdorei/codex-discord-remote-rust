//! One admitted recovery command owns one effect sequence, never a retry lease.
use super::{IngressKind, StoredIngress, read::get_in};
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub struct RecoveryClaim {
    record: StoredIngress,
}

fn refused() -> StoreError {
    StoreError::Integrity(
        "recovery admission changed or was already used; no retarget or replay".into(),
    )
}

pub fn validate_recovery_binding_in(db: &Connection, binding: &Value, channel: i64) -> Result<()> {
    let target = binding
        .get("target")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(refused)?;
    let command = binding
        .get("command")
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    if command.len() != 1 || channel <= 0 {
        return Err(refused());
    }
    let fields = command
        .get("Recover")
        .or_else(|| command.get("Repair"))
        .and_then(Value::as_object)
        .ok_or_else(refused)?;
    if fields.len() != 1 {
        return Err(refused());
    }
    let reference = fields.get("reference").ok_or_else(refused)?;
    let explicit = reference.as_str().is_some_and(|v| !v.trim().is_empty());
    if !explicit && !reference.is_null() {
        return Err(refused());
    }
    let valid = match binding.get("route").and_then(Value::as_str) {
        Some("Explicit") => explicit,
        Some("Mapped") if !explicit => {
            crate::mapping::mirrored_thread_id_in(db, Some(channel))?.as_deref() == Some(target)
        }
        Some("Selected") if !explicit => {
            // The runtime additionally checks its exact selected-thread snapshot.
            crate::mapping::mirrored_thread_id_in(db, Some(channel))?.is_none()
        }
        _ => false,
    };
    if !valid {
        return Err(refused());
    }
    Ok(())
}

fn validate_record(db: &Connection, record: &StoredIngress) -> Result<()> {
    let event = record.event_id.filter(|v| *v > 0).ok_or_else(refused)?;
    let binding = record
        .payload
        .get("lifecycle_binding")
        .ok_or_else(refused)?;
    if record.kind != IngressKind::Message
        || record.ingress_id != format!("message:{event}")
        || record.source_message_id != Some(event)
        || record.owner_user_id <= 0
        || record.owner_id.is_some()
        || record.owner_kind.is_some()
        || record.payload.get("version") != Some(&serde_json::json!(1))
        || record.payload.pointer("/plan/Execute") != binding.get("command")
        || record.target_thread_id.as_deref() != binding.get("target").and_then(Value::as_str)
    {
        return Err(refused());
    }
    validate_recovery_binding_in(db, binding, record.channel_id)
}

pub fn claim_recovery(path: &Path, expected: &StoredIngress) -> Result<RecoveryClaim> {
    let mut db = open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut record = get_in(&tx, &expected.ingress_id)?.ok_or_else(refused)?;
    if record != *expected || record.state != "executing" || record.phase != "processing" {
        return Err(refused());
    }
    validate_record(&tx, &record)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    if tx.execute(
        "UPDATE discord_ingress_journal SET phase='recovery_claimed',updated_at=?
        WHERE ingress_id=? AND state='executing' AND phase='processing' AND owner_id IS NULL",
        params![now, record.ingress_id],
    )? != 1
    {
        return Err(refused());
    }
    record.phase = "recovery_claimed".into();
    record.updated_at = now;
    tx.commit()?;
    Ok(RecoveryClaim { record })
}

impl RecoveryClaim {
    pub fn validate(&self, path: &Path) -> Result<()> {
        self.validate_in(&open_initialized(path)?)
    }

    /// Also used inside the IMMEDIATE request-cancellation transaction.
    pub fn validate_in(&self, db: &Connection) -> Result<()> {
        if get_in(db, &self.record.ingress_id)?.as_ref() != Some(&self.record) {
            return Err(refused());
        }
        validate_record(db, &self.record)
    }
}

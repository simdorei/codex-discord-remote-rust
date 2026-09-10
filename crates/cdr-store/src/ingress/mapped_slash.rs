use super::{IngressAdmission, IngressKind, NewIngress, StoredIngress};
use crate::{Result, StoreError};
use rusqlite::TransactionBehavior;
use serde_json::Value;
use std::path::Path;

/// Resolve and persist the original mapping under the same write transaction.
/// Unknown/unmapped/New targets remain unknown; no selected-target guess is made.
pub fn admit_mapped_slash_prompt(path: &Path, request: &NewIngress) -> Result<IngressAdmission> {
    if !supported(request.kind, &request.payload) || request.target_thread_id.is_some() {
        return Err(StoreError::Integrity(
            "unsupported mapped slash admission envelope".into(),
        ));
    }
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut bound = request.clone();
    bound.target_thread_id = crate::mapping::mirrored_thread_id_in(&tx, Some(request.channel_id))?;
    let admitted = super::admission::admit_in(&tx, &bound)?;
    tx.commit()?;
    Ok(admitted)
}

#[must_use]
pub fn frozen_slash_target(record: &StoredIngress) -> Option<&str> {
    supported(record.kind, &record.payload)
        .then_some(record.target_thread_id.as_deref())
        .flatten()
}

fn supported(kind: IngressKind, payload: &Value) -> bool {
    kind == IngressKind::Interaction
        && payload.get("version").and_then(Value::as_u64) == Some(1)
        && matches!(
            payload.pointer("/work/Slash/name").and_then(Value::as_str),
            Some("ask" | "interview")
        )
        && payload
            .pointer("/work/Slash/values/prompt/String")
            .is_some_and(Value::is_string)
}

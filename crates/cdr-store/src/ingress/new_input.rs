//! Immutable attachment preparation; the original Discord envelope stays intact.
use super::{IngressKind, StoredIngress, new_command_prompt, read::get_in};
use crate::{Result, StoreError};
use rusqlite::{TransactionBehavior, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn envelope_digest(record: &StoredIngress) -> Result<String> {
    Ok(digest(&serde_json::to_vec(&(
        &record.ingress_id,
        record.kind,
        record.event_id,
        record.channel_id,
        record.owner_user_id,
        &record.payload,
    ))?))
}

fn has_attachments(record: &StoredIngress) -> bool {
    record.kind == IngressKind::Message
        && record.payload["attachments"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
}

/// Only a successfully prepared, exact-source attachment input may replace the
/// raw prompt for execution. Text-only and non-New requests retain their contract.
pub fn new_execution_prompt(record: &StoredIngress) -> Result<Option<&str>> {
    let Some(raw) = new_command_prompt(record) else {
        return Ok(None);
    };
    let prepared = record.outcome.as_ref().and_then(|v| v.get("new_input"));
    if !has_attachments(record) {
        if prepared.is_some() {
            return Err(StoreError::Integrity(
                "prepared new attachment envelope changed".into(),
            ));
        }
        return Ok(Some(raw));
    }
    let prepared = prepared.ok_or_else(|| {
        StoreError::Integrity(
            "new attachments have no durable prepared input; no execution permitted".into(),
        )
    })?;
    let prompt = prepared["prompt"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| StoreError::Integrity("new prepared input is missing".into()))?;
    if prepared["version"] != 1
        || prepared["source_sha256"] != envelope_digest(record)?
        || prepared["prompt_sha256"] != digest(prompt.as_bytes())
    {
        return Err(StoreError::Integrity(
            "new prepared input or original envelope changed".into(),
        ));
    }
    Ok(Some(prompt))
}

pub fn record_new_input(path: &Path, key: &str, raw: &str, prompt: &str, now: f64) -> Result<()> {
    if !now.is_finite() || prompt.trim().is_empty() {
        return Err(StoreError::Integrity(
            "invalid new input preparation".into(),
        ));
    }
    let mut connection = crate::schema::open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let record = get_in(&tx, key)?
        .ok_or_else(|| StoreError::Integrity("new input owner is missing".into()))?;
    if new_command_prompt(&record) != Some(raw) || !has_attachments(&record) {
        return Err(StoreError::Integrity(
            "new input preparation does not own the original prompt and attachments".into(),
        ));
    }
    let prepared = json!({"version":1,"source_sha256":envelope_digest(&record)?,
        "prompt_sha256":digest(prompt.as_bytes()),"prompt":prompt});
    if let Some(existing) = record.outcome.as_ref().and_then(|v| v.get("new_input")) {
        if existing != &prepared {
            return Err(StoreError::Integrity(
                "new prepared input is immutable".into(),
            ));
        }
        return Ok(());
    }
    if record.state != "executing" || record.phase != "processing" {
        return Err(StoreError::Integrity(
            "new attachment input must be frozen before thread/start".into(),
        ));
    }
    tx.execute("UPDATE discord_ingress_journal SET outcome_json=json_set(COALESCE(outcome_json,'{}'),'$.new_input',json(?)),updated_at=? WHERE ingress_id=?",
        params![prepared.to_string(), now, key])?;
    tx.commit()?;
    Ok(())
}

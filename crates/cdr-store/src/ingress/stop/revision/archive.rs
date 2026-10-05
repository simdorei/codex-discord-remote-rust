//! Archive-only derived scope. Never relax the ordinary exact-target validator.
use crate::{Result, StoreError};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn validate_in(
    db: &Connection,
    method: &str,
    target: Option<&str>,
    origin: &Value,
) -> Result<()> {
    if db.is_autocommit() {
        return Err(StoreError::ActiveTransaction);
    }
    let root = origin
        .get("target")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.trim() == *s)
        .ok_or_else(super::refused)?;
    let revision = origin
        .get("stopRevision")
        .and_then(Value::as_i64)
        .ok_or_else(super::refused)?;
    let scope = origin
        .get("archiveTargets")
        .and_then(Value::as_array)
        .filter(|members| !members.is_empty() && members.len() <= 101)
        .ok_or_else(super::refused)?;
    if origin.as_object().is_none_or(|object| object.len() != 3) {
        return Err(super::refused());
    }
    let mut members = BTreeSet::new();
    for member in scope {
        let member = member
            .as_str()
            .filter(|s| !s.is_empty() && s.trim() == *s)
            .ok_or_else(super::refused)?;
        if !members.insert(member) {
            return Err(super::refused());
        }
    }
    if !members.contains(root)
        || target.is_none_or(|target| !members.contains(target))
        || !matches!(method, "thread/resume" | "thread/archive")
        || (method == "thread/archive" && target != Some(root))
    {
        return Err(super::refused());
    }
    // All members use the original revision in this same writer transaction.
    // In particular a child stop also revokes the final root archive.
    for member in members {
        super::validate_in(
            db,
            Some(member),
            Some(&json!({
                "target":member,"stopRevision":revision,
            })),
        )?;
    }
    Ok(())
}

//! Durable archive scope, serialized with ingress and prompt ownership writes.
//! No lease expiry or automatic replay: an interrupted dispatch stays fenced.
mod schema;
pub(crate) use schema::{migrate_schema, schema_current};

use crate::{Result, StoreError};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::{collections::BTreeSet, path::Path};

pub fn reserve(path: &Path, scope: &BTreeSet<String>, own: Option<&str>) -> Result<String> {
    if scope.is_empty() || scope.iter().any(|id| id.is_empty() || id.trim() != id) {
        return Err(StoreError::Integrity(
            "invalid archive reservation scope".into(),
        ));
    }
    let mut db = crate::schema::open_initialized(path)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for target in scope {
        let blocked: Option<String> = tx
            .query_row(
                "SELECT ingress_id FROM discord_ingress_journal
             WHERE (target_thread_id=?1 OR target_thread_id IS NULL)
             AND state!='completed' AND NOT(state='owned' AND confirmation_delivered=1)
             AND (?2 IS NULL OR ingress_id!=?2) ORDER BY created_at,ingress_id LIMIT 1",
                params![target, own],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(key) = blocked {
            return Err(StoreError::Integrity(format!(
                "archive reservation refused: unfinished request {key} is preserved; no archive was sent"
            )));
        }
        let occupied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)
             OR EXISTS(SELECT 1 FROM codex_turn_queue WHERE target_thread_id=?1)
             OR EXISTS(SELECT 1 FROM codex_prompt_intakes WHERE target_thread_id=?1)",
            [target],
            |row| row.get(0),
        )?;
        if occupied {
            return Err(StoreError::Integrity(format!(
                "archive reservation refused: {target} has work or an existing archive fence; no archive was sent"
            )));
        }
    }
    let operation = uuid::Uuid::new_v4().to_string();
    for target in scope {
        tx.execute(
            "INSERT INTO codex_archive_fences (target_thread_id,operation_id,own_ingress_id,phase)
             VALUES (?,?,?,'attempted')",
            params![target, operation, own],
        )?;
    }
    tx.commit()?;
    Ok(operation)
}

pub fn target_is_fenced(path: &Path, target_thread_id: &str) -> Result<bool> {
    Ok(crate::schema::open_initialized(path)?.query_row(
        "SELECT EXISTS(SELECT 1 FROM codex_archive_fences WHERE target_thread_id=?1)",
        [target_thread_id],
        |row| row.get(0),
    )?)
}

/// Only after every scope member's persisted archived state was verified.
/// The target fence and all held requests remain; no implicit unarchive/replay.
pub fn verified(path: &Path, operation: &str) -> Result<()> {
    let db = crate::schema::open_initialized(path)?;
    if db.execute("UPDATE codex_archive_fences SET phase='verified' WHERE operation_id=? AND phase='attempted'", [operation])? == 0 {
        return Err(StoreError::Integrity("missing attempted archive reservation".into()));
    }
    Ok(())
}

/// Call only for a proven pre-effect rejection, never a timeout or disconnect.
/// This releases this operation's reservation, not the separately held ingress.
pub fn release_rejected(path: &Path, operation: &str) -> Result<()> {
    let db = crate::schema::open_initialized(path)?;
    if db.execute(
        "DELETE FROM codex_archive_fences WHERE operation_id=? AND phase='attempted'",
        [operation],
    )? == 0
    {
        return Err(StoreError::Integrity(
            "missing rejected archive reservation".into(),
        ));
    }
    Ok(())
}

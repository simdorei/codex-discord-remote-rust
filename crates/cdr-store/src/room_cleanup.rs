//! Durable room-close boundary; late ingress remains saved, never silently executed.
use crate::{Result, StoreError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
pub mod archive;
mod archive_schema;
pub mod archived_rejections;
mod pending;
mod schema;
pub(crate) use schema::{migrate_schema, schema_current};

pub fn begin(path: &Path, channel: i64, target: Option<&str>, now: f64) -> Result<String> {
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let token = begin_in(&transaction, channel, target, now, &[])?;
    transaction.commit()?;
    Ok(token)
}

fn begin_in(
    connection: &Connection,
    channel: i64,
    target: Option<&str>,
    now: f64,
    excluded: &[String],
) -> Result<String> {
    let fenced: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE channel_id=?)",
        [channel],
        |r| r.get(0),
    )?;
    if fenced {
        return Err(StoreError::Integrity(format!(
            "room {channel} cleanup outcome is fenced; manual reconciliation required"
        )));
    }
    let mapped = connection.prepare("SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id=? OR discord_channel_id=?")?
        .query_map([channel,channel],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let matches = match (target, mapped.as_slice()) {
        (Some(expected), [actual]) => expected == actual,
        (None, []) => true,
        _ => false,
    };
    if channel <= 0 || !now.is_finite() || !matches {
        return Err(StoreError::Integrity(
            "cleanup mapping identity changed or invalid".into(),
        ));
    }
    if let Some(thread) = target {
        crate::idle_release::before_cleanup(connection, thread)?;
    }
    if let Some(reason) =
        pending::reason_with_exclusions(connection, channel, target, None, false, excluded)?
    {
        return Err(StoreError::CleanupProtected { channel, reason });
    }
    let token = uuid::Uuid::new_v4().to_string();
    connection.execute("INSERT INTO cdr_cleanup_fences (channel_id,target_thread_id,token,phase,created_at) VALUES (?,?,?,'deleting',?)",params![channel,target,token,now])?;
    Ok(token)
}

/// Successful deletion keeps a tombstone so delayed events cannot reopen execution.
pub fn complete(path: &Path, channel: i64, token: &str) -> Result<()> {
    let connection = crate::schema::open_initialized(path)?;
    if connection.execute("UPDATE cdr_cleanup_fences SET phase='deleted' WHERE channel_id=? AND token=? AND phase='deleting'",params![channel,token])? != 1 {
        return Err(StoreError::Integrity("cleanup fence ownership changed".into()));
    }
    Ok(())
}

/// Only an authoritative rejection that could not delete the room may release it.
pub fn release_rejected(path: &Path, channel: i64, token: &str) -> Result<()> {
    let connection = crate::schema::open_initialized(path)?;
    if connection.execute(
        "DELETE FROM cdr_cleanup_fences WHERE channel_id=? AND token=? AND phase='deleting'",
        params![channel, token],
    )? != 1
    {
        return Err(StoreError::Integrity(
            "cleanup fence ownership changed".into(),
        ));
    }
    Ok(())
}

pub fn phase(path: &Path, channel: i64) -> Result<Option<String>> {
    Ok(crate::schema::open_initialized(path)?
        .query_row(
            "SELECT phase FROM cdr_cleanup_fences WHERE channel_id=?",
            [channel],
            |r| r.get(0),
        )
        .optional()?)
}

/// A completed tombstone can verify a retry only for its original target.
pub fn confirmed_target(path: &Path, channel: i64, target: &str) -> Result<bool> {
    Ok(crate::schema::open_initialized(path)?.query_row(
        "SELECT EXISTS(SELECT 1 FROM cdr_cleanup_fences WHERE channel_id=? AND target_thread_id=? AND phase='deleted')",
        params![channel,target], |r| r.get(0),
    )?)
}

pub fn unconfirmed_channels(path: &Path) -> Result<Vec<i64>> {
    let connection = crate::schema::open_initialized(path)?;
    Ok(connection
        .prepare(
            "SELECT channel_id FROM cdr_cleanup_fences WHERE phase='deleting' ORDER BY channel_id",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn pending_reason(
    path: &Path,
    channel: i64,
    target: Option<&str>,
) -> Result<Option<&'static str>> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let transaction = connection.unchecked_transaction()?;
    pending::reason(&transaction, channel, target)
}

/// Read-only deployment preflight for the pre-commentary-outbox baseline.
/// Only that known optional table may be absent; every other schema error fails.
pub fn pending_reason_pre_commentary_schema(
    path: &Path,
    channel: i64,
    target: Option<&str>,
) -> Result<Option<&'static str>> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let transaction = connection.unchecked_transaction()?;
    pending::reason_for_schema(&transaction, channel, target, None, true)
}

use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::Result;
use crate::schema::open_initialized;

mod context;
mod origin;
pub use context::{get_cursor_turn, update_cursor_with_turn};
pub(crate) use context::{migrate_schema, schema_current};
pub(crate) use origin::record_job_origin;
pub use origin::{record_user_origin, user_origin_marker};

#[derive(Clone, Debug, PartialEq)]
pub struct MirrorOffset {
    pub rollout_path: String,
    pub cursor: i64,
    pub updated_at: f64,
}

pub fn claim_event(path: &Path, event_digest: &str, thread_id: &str, now: f64) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "INSERT OR IGNORE INTO codex_session_mirror_events \
         (event_digest, codex_thread_id, created_at) VALUES (?, ?, ?)",
        params![event_digest, thread_id, now],
    )? == 1)
}

pub fn has_event(path: &Path, event_digest: &str, thread_id: &str) -> Result<bool> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT 1 FROM codex_session_mirror_events \
             WHERE event_digest = ? AND codex_thread_id = ?",
            params![event_digest, thread_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

#[must_use]
pub fn turn_origin_marker(thread_id: &str, turn_id: &str) -> String {
    format!("discord-origin:v1:{thread_id}:{turn_id}")
}

pub fn cleanup_events(path: &Path, retention_seconds: f64, now: f64) -> Result<usize> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM codex_session_mirror_events WHERE created_at < ?",
        [now - retention_seconds],
    )?)
}

pub fn get_or_init_cursor(
    path: &Path,
    thread_id: &str,
    rollout_path: &str,
    initial_cursor: i64,
    now: f64,
) -> Result<i64> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current = transaction
        .query_row(
            "SELECT rollout_path, cursor FROM codex_session_mirror_offsets \
             WHERE codex_thread_id = ?",
            [thread_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let cursor = if let Some((stored_path, cursor)) = current {
        if stored_path == rollout_path {
            cursor
        } else {
            replace_offset(&transaction, thread_id, rollout_path, initial_cursor, now)?;
            initial_cursor
        }
    } else {
        replace_offset(&transaction, thread_id, rollout_path, initial_cursor, now)?;
        initial_cursor
    };
    transaction.commit()?;
    Ok(cursor)
}

pub fn get_offset(path: &Path, thread_id: &str) -> Result<Option<MirrorOffset>> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT rollout_path, cursor, updated_at FROM codex_session_mirror_offsets \
             WHERE codex_thread_id = ?",
            [thread_id],
            |row| {
                Ok(MirrorOffset {
                    rollout_path: row.get(0)?,
                    cursor: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()?)
}

pub fn update_cursor(
    path: &Path,
    thread_id: &str,
    rollout_path: &str,
    cursor: i64,
    now: f64,
) -> Result<()> {
    let connection = open_initialized(path)?;
    replace_offset(&connection, thread_id, rollout_path, cursor, now)
}

fn replace_offset(
    connection: &rusqlite::Connection,
    thread_id: &str,
    rollout_path: &str,
    cursor: i64,
    now: f64,
) -> Result<()> {
    connection.execute(
        "INSERT OR REPLACE INTO codex_session_mirror_offsets \
         (codex_thread_id, rollout_path, cursor, updated_at) VALUES (?, ?, ?, ?)",
        params![thread_id, rollout_path, cursor, now],
    )?;
    Ok(())
}

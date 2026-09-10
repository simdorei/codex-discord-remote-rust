use crate::{Result, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    let mut statement = connection.prepare("PRAGMA table_info(codex_session_mirror_offsets)")?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "turn_context"))
}

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS codex_session_mirror_offsets
        (codex_thread_id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, cursor INTEGER NOT NULL,
         updated_at REAL NOT NULL, turn_context TEXT)",
        [],
    )?;
    if !schema_current(connection)? {
        connection.execute(
            "ALTER TABLE codex_session_mirror_offsets ADD COLUMN turn_context TEXT",
            [],
        )?;
    }
    Ok(())
}

/// NULL means a pre-upgrade cursor; an empty string is an initialized cursor without a turn.
pub fn get_cursor_turn(path: &Path, thread: &str) -> Result<Option<String>> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT turn_context FROM codex_session_mirror_offsets WHERE codex_thread_id = ?",
            [thread],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

pub fn update_cursor_with_turn(
    path: &Path,
    thread: &str,
    rollout: &str,
    cursor: i64,
    now: f64,
    turn: Option<&str>,
) -> Result<()> {
    open_initialized(path)?.execute(
        "INSERT OR REPLACE INTO codex_session_mirror_offsets
         (codex_thread_id, rollout_path, cursor, updated_at, turn_context) VALUES (?, ?, ?, ?, ?)",
        params![thread, rollout, cursor, now, turn.unwrap_or_default()],
    )?;
    Ok(())
}

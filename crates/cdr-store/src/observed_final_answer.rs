//! Exact final-answer events retained until terminal completion is staged.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::Result;
use crate::schema::open_initialized;

pub(crate) fn migrate_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS codex_observed_final_answers (
        thread_id TEXT NOT NULL, turn_id TEXT NOT NULL, generation INTEGER NOT NULL,
        content TEXT NOT NULL,
        PRIMARY KEY(thread_id, turn_id, generation));",
    )?;
    Ok(())
}

pub(crate) fn schema_current(connection: &Connection) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table'
        AND name='codex_observed_final_answers')",
        [],
        |row| row.get(0),
    )?)
}

/// Records only a final belonging to the exact running queue owner and generation.
pub fn record(
    path: &Path,
    thread: &str,
    turn: &str,
    generation: i64,
    content: &str,
) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "INSERT OR IGNORE INTO codex_observed_final_answers
        (thread_id,turn_id,generation,content)
        SELECT ?,?,?,? WHERE EXISTS(SELECT 1 FROM codex_turn_queue
        WHERE target_thread_id=? AND turn_id=? AND app_server_generation=? AND state='running')",
        params![thread, turn, generation, content, thread, turn, generation],
    )? == 1)
}

pub fn get(path: &Path, thread: &str, turn: &str, generation: i64) -> Result<Option<String>> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT content FROM codex_observed_final_answers
            WHERE thread_id=? AND turn_id=? AND generation=?",
            params![thread, turn, generation],
            |row| row.get(0),
        )
        .optional()?)
}

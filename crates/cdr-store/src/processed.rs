use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::Result;
use crate::schema::open_initialized;

pub fn claim(path: &Path, message_id: i64, now: f64) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "INSERT OR IGNORE INTO discord_processed_messages (message_id, seen_at) VALUES (?, ?)",
        params![message_id, now],
    )? == 1)
}

pub fn is_processed(path: &Path, message_id: i64) -> Result<bool> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT 1 FROM discord_processed_messages WHERE message_id = ?",
            [message_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

pub fn mark(path: &Path, message_id: i64, now: f64) -> Result<()> {
    open_initialized(path)?.execute(
        "INSERT OR REPLACE INTO discord_processed_messages (message_id, seen_at) VALUES (?, ?)",
        params![message_id, now],
    )?;
    Ok(())
}

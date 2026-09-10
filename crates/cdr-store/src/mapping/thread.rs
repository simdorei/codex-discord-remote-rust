use std::path::Path;

use rusqlite::{OptionalExtension, params};

use crate::Result;
use crate::schema::open_initialized;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MirrorTarget {
    pub codex_thread_id: String,
    pub thread_title: String,
    pub discord_channel_id: i64,
    pub discord_thread_id: i64,
}

pub fn upsert_thread(
    path: &Path,
    thread_id: &str,
    project_key: &str,
    title: &str,
    project_channel_id: i64,
    discord_thread_id: i64,
    now: f64,
) -> Result<()> {
    open_initialized(path)?.execute(
        "INSERT INTO mirror_threads (codex_thread_id, project_key, thread_title, \
         discord_channel_id, discord_thread_id, updated_at) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(codex_thread_id) DO UPDATE SET project_key = excluded.project_key, \
         thread_title = excluded.thread_title, discord_channel_id = excluded.discord_channel_id, \
         discord_thread_id = excluded.discord_thread_id, updated_at = excluded.updated_at",
        params![
            thread_id,
            project_key,
            title,
            project_channel_id,
            discord_thread_id,
            now,
        ],
    )?;
    Ok(())
}

pub fn mirrored_thread_id(path: &Path, channel_id: Option<i64>) -> Result<Option<String>> {
    if channel_id.is_none_or(|value| value == 0) {
        return Ok(None);
    }
    mirrored_thread_id_in(&open_initialized(path)?, channel_id)
}

pub(crate) fn mirrored_thread_id_in(
    connection: &rusqlite::Connection,
    channel_id: Option<i64>,
) -> Result<Option<String>> {
    let Some(channel_id) = channel_id.filter(|value| *value != 0) else {
        return Ok(None);
    };
    let exact_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM mirror_threads WHERE discord_thread_id=?",
        [channel_id],
        |row| row.get(0),
    )?;
    if exact_count > 1 {
        return Err(crate::StoreError::Integrity(format!(
            "Discord room {channel_id} is mapped to multiple Codex threads; routing refused"
        )));
    }
    let exact = connection
        .query_row(
            "SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id = ?",
            [channel_id],
            |row| row.get(0),
        )
        .optional()?;
    if exact.is_some() {
        return Ok(exact);
    }
    let mut statement = connection.prepare(
        "SELECT codex_thread_id FROM mirror_threads WHERE discord_channel_id = ? \
         ORDER BY updated_at DESC LIMIT 2",
    )?;
    let rows = statement
        .query_map([channel_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows.len() == 1).then(|| rows[0].clone()))
}

pub fn thread_channels(path: &Path, thread_id: &str) -> Result<Option<(i64, i64)>> {
    Ok(open_initialized(path)?
        .query_row(
            "SELECT discord_channel_id, discord_thread_id FROM mirror_threads \
             WHERE codex_thread_id = ?",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn update_discord_thread_id(
    path: &Path,
    thread_id: &str,
    discord_thread_id: i64,
    now: f64,
) -> Result<Option<(i64, i64)>> {
    let connection = open_initialized(path)?;
    let previous = connection
        .query_row(
            "SELECT discord_channel_id, discord_thread_id FROM mirror_threads \
             WHERE codex_thread_id = ?",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if previous.is_some() {
        connection.execute(
            "UPDATE mirror_threads SET discord_thread_id = ?, updated_at = ? \
             WHERE codex_thread_id = ?",
            params![discord_thread_id, now, thread_id],
        )?;
    }
    Ok(previous)
}

pub fn mirror_targets(path: &Path, limit: i64) -> Result<Vec<MirrorTarget>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT codex_thread_id, thread_title, discord_channel_id, discord_thread_id \
         FROM mirror_threads ORDER BY updated_at DESC LIMIT ?",
    )?;
    let rows = statement
        .query_map([limit], |row| {
            Ok(MirrorTarget {
                codex_thread_id: row.get(0)?,
                thread_title: row.get(1)?,
                discord_channel_id: row.get(2)?,
                discord_thread_id: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|row| !row.codex_thread_id.is_empty() && row.discord_thread_id != 0)
        .collect())
}

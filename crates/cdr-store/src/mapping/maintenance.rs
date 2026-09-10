use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::Result;
use crate::schema::open_initialized;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaleThread {
    pub thread_id: String,
    pub discord_thread_id: i64,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaleProject {
    pub project_key: String,
    pub project_name: String,
    pub discord_channel_id: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchivedDeleteCounts {
    pub mirror_threads: usize,
    pub session_mirror_offsets: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemainingDiscordIds {
    pub thread_ids: BTreeSet<i64>,
    pub project_channel_ids: Vec<i64>,
}

pub fn stale_threads(
    path: &Path,
    valid_ids: &BTreeSet<String>,
    updated_before: f64,
) -> Result<Vec<StaleThread>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT codex_thread_id, discord_thread_id, thread_title, updated_at FROM mirror_threads",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                StaleThread {
                    thread_id: row.get(0)?,
                    discord_thread_id: row.get(1)?,
                    title: row.get(2)?,
                },
                row.get::<_, f64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(row, updated)| !valid_ids.contains(&row.thread_id) && *updated < updated_before)
        .map(|(row, _)| row)
        .collect())
}

pub fn stale_projects(
    path: &Path,
    valid_keys: &BTreeSet<String>,
    updated_before: f64,
) -> Result<Vec<StaleProject>> {
    let connection = open_initialized(path)?;
    let mut statement = connection.prepare(
        "SELECT project_key, project_name, discord_channel_id, updated_at FROM mirror_projects",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                StaleProject {
                    project_key: row.get(0)?,
                    project_name: row.get(1)?,
                    discord_channel_id: row.get(2)?,
                },
                row.get::<_, f64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|(row, updated)| {
            !valid_keys.contains(&row.project_key) && *updated < updated_before
        })
        .map(|(row, _)| row)
        .collect())
}

pub fn delete_stale(
    path: &Path,
    valid_threads: &BTreeSet<String>,
    valid_projects: &BTreeSet<String>,
    updated_before: f64,
) -> Result<()> {
    let threads = stale_threads(path, valid_threads, updated_before)?;
    let projects = stale_projects(path, valid_projects, updated_before)?;
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for row in threads {
        transaction.execute(
            "DELETE FROM mirror_threads WHERE codex_thread_id = ? AND updated_at < ?",
            params![row.thread_id, updated_before],
        )?;
    }
    for row in projects {
        transaction.execute(
            "DELETE FROM mirror_projects WHERE project_key = ? AND updated_at < ?",
            params![row.project_key, updated_before],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn delete_archived_state(path: &Path, thread_id: &str) -> Result<ArchivedDeleteCounts> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mirror_threads = transaction.execute(
        "DELETE FROM mirror_threads WHERE codex_thread_id = ?",
        [thread_id],
    )?;
    let session_mirror_offsets = transaction.execute(
        "DELETE FROM codex_session_mirror_offsets WHERE codex_thread_id = ?",
        [thread_id],
    )?;
    transaction.commit()?;
    Ok(ArchivedDeleteCounts {
        mirror_threads,
        session_mirror_offsets,
    })
}

pub fn remaining_discord_ids(path: &Path) -> Result<RemainingDiscordIds> {
    let connection = open_initialized(path)?;
    let thread_ids = collect_ids(&connection, "SELECT discord_thread_id FROM mirror_threads")?
        .into_iter()
        .collect();
    let project_channel_ids = collect_ids(
        &connection,
        "SELECT discord_channel_id FROM mirror_projects",
    )?;
    Ok(RemainingDiscordIds {
        thread_ids,
        project_channel_ids,
    })
}

fn collect_ids(connection: &rusqlite::Connection, sql: &str) -> Result<Vec<i64>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([], |row| row.get::<_, Option<i64>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .filter(|value| *value != 0)
        .collect())
}

pub fn is_mirrored_channel(path: &Path, channel_id: Option<i64>) -> Result<bool> {
    let Some(channel_id) = channel_id else {
        return Ok(false);
    };
    Ok(open_initialized(path)?
        .query_row(
            "SELECT 1 FROM mirror_threads WHERE discord_thread_id = ? OR discord_channel_id = ? \
             UNION ALL SELECT 1 FROM mirror_projects WHERE discord_channel_id = ? LIMIT 1",
            params![channel_id, channel_id, channel_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

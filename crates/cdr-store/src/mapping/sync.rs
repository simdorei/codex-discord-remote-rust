use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::path::Path;

#[derive(Clone, Copy)]
pub struct MirrorThreadUpdate<'a> {
    pub thread_id: &'a str,
    pub project_key: &'a str,
    pub title: &'a str,
    pub parent_id: i64,
    pub channel_id: i64,
    pub now: f64,
}

pub fn commit_thread_sync(
    path: &Path,
    update: MirrorThreadUpdate<'_>,
    expected: Option<(i64, i64)>,
) -> Result<()> {
    commit_checked(path, update, expected, None, false)
}

pub fn commit_new_thread_sync(
    path: &Path,
    update: MirrorThreadUpdate<'_>,
    expected: Option<(i64, i64)>,
    project_key: Option<&str>,
) -> Result<()> {
    commit_checked(path, update, expected, project_key, true)
}

fn commit_checked(
    path: &Path,
    update: MirrorThreadUpdate<'_>,
    expected: Option<(i64, i64)>,
    project_key: Option<&str>,
    check_project: bool,
) -> Result<()> {
    let mut connection = open_initialized(path)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if check_project {
        let keys = tx
            .prepare("SELECT project_key FROM mirror_projects WHERE discord_channel_id=?")?
            .query_map([update.parent_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if keys.iter().map(String::as_str).collect::<Vec<_>>()
            != project_key.into_iter().collect::<Vec<_>>()
        {
            return Err(StoreError::Integrity(format!(
                "new-thread project mapping changed for channel {}; created room {} retained without attaching or retrying",
                update.parent_id, update.channel_id
            )));
        }
    }
    let occupied: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM mirror_threads WHERE discord_thread_id=? AND codex_thread_id<>?)",
        params![update.channel_id, update.thread_id], |row| row.get(0))?;
    if occupied {
        return Err(StoreError::Integrity(format!(
            "Discord room {} already belongs to another Codex thread; mapping unchanged",
            update.channel_id
        )));
    }
    let actual = tx.query_row("SELECT discord_channel_id, discord_thread_id FROM mirror_threads WHERE codex_thread_id = ?",
        [update.thread_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).optional()?;
    if actual != expected {
        return Err(StoreError::MirrorMappingChanged {
            discord_channel_id: update.channel_id,
            expected_target_thread_id: update.thread_id.to_owned(),
            actual_target_thread_id: None,
        });
    }
    tx.execute("INSERT INTO mirror_threads (codex_thread_id, project_key, thread_title, discord_channel_id, discord_thread_id, updated_at)
        VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(codex_thread_id) DO UPDATE SET
        project_key=excluded.project_key, thread_title=excluded.thread_title, discord_channel_id=excluded.discord_channel_id,
        discord_thread_id=excluded.discord_thread_id, updated_at=excluded.updated_at",
        params![update.thread_id, update.project_key, update.title, update.parent_id, update.channel_id, update.now])?;
    tx.commit()?;
    Ok(())
}

pub fn retire_thread_sync(
    path: &Path,
    thread: &str,
    channel: i64,
    updated_before: f64,
) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM mirror_threads WHERE codex_thread_id = ?
        AND discord_thread_id = ? AND updated_at < ?",
        params![thread, channel, updated_before],
    )? == 1)
}

pub fn retire_project_sync(path: &Path, key: &str, channel: i64, started: f64) -> Result<bool> {
    Ok(open_initialized(path)?.execute(
        "DELETE FROM mirror_projects WHERE project_key=? AND discord_channel_id=? AND updated_at < ?
         AND NOT EXISTS (SELECT 1 FROM mirror_threads WHERE discord_channel_id=?)",
        params![key, channel, started, channel],
    )? == 1)
}

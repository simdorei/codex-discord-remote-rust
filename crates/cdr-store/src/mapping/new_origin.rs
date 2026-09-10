//! Semantic routing snapshot for a new conversation. Titles/timestamps are not
//! routing authority. Capture under one read transaction; compare under the
//! creation writer transaction before permitting any remote creation.
use crate::{Result, StoreError};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NewThreadOrigin {
    pub version: u8,
    pub channel: i64,
    pub target: Option<String>,
    pub mapped_project: Option<String>,
    pub parent_channel: Option<i64>,
    pub project: Option<String>,
    pub parent_project: Option<String>,
    pub chat_targets: Vec<String>,
}

pub fn new_thread_origin(path: &Path, channel: i64) -> Result<NewThreadOrigin> {
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction()?;
    let origin = new_thread_origin_in(&transaction, channel)?;
    transaction.commit()?;
    Ok(origin)
}

pub(crate) fn new_thread_origin_in(
    connection: &Connection,
    channel: i64,
) -> Result<NewThreadOrigin> {
    let target = super::mirrored_thread_id_in(connection, Some(channel))?;
    let mapped: Option<(String, i64)> = target
        .as_deref()
        .map(|id| {
            connection.query_row(
        "SELECT project_key,discord_channel_id FROM mirror_threads WHERE codex_thread_id=?",[id],
        |row|Ok((row.get(0)?,row.get(1)?))).optional()
        })
        .transpose()?
        .flatten();
    let project = project_key(connection, channel)?;
    let parent_project = mapped
        .as_ref()
        .map(|(_, parent)| project_key(connection, *parent))
        .transpose()?
        .flatten();
    let chat_targets = if target.is_none()
        && project
            .as_deref()
            .is_some_and(|key| key == "codex:chats" || key.starts_with("projectless:"))
    {
        let mut statement = connection.prepare("SELECT codex_thread_id FROM mirror_threads WHERE discord_channel_id=? ORDER BY codex_thread_id")?;
        statement
            .query_map([channel], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok(NewThreadOrigin {
        version: 1,
        channel,
        target,
        mapped_project: mapped.as_ref().map(|(p, _)| p.clone()),
        parent_channel: mapped.map(|(_, p)| p),
        project,
        parent_project,
        chat_targets,
    })
}

fn project_key(connection: &Connection, channel: i64) -> Result<Option<String>> {
    let mut statement = connection.prepare("SELECT project_key FROM mirror_projects WHERE discord_channel_id=? ORDER BY project_key LIMIT 2")?;
    let rows = statement
        .query_map([channel], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    if rows.len() > 1 {
        return Err(StoreError::Integrity(
            "new origin has multiple project mappings".into(),
        ));
    }
    Ok(rows.into_iter().next())
}

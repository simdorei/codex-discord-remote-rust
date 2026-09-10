use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::ProjectMatch;
use crate::Result;
use crate::schema::open_initialized;

pub fn upsert_project<F>(
    path: &Path,
    canonical_key: &str,
    project_name: &str,
    channel_id: i64,
    now: f64,
    keys_match: F,
) -> Result<Vec<String>>
where
    F: Fn(&str, &str) -> bool,
{
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let aliases = {
        let mut statement = transaction.prepare("SELECT project_key FROM mirror_projects")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|key| key != canonical_key && keys_match(key, canonical_key))
            .collect::<Vec<_>>()
    };
    for alias in &aliases {
        transaction.execute(
            "UPDATE mirror_threads SET project_key = ? WHERE project_key = ?",
            params![canonical_key, alias],
        )?;
        transaction.execute("DELETE FROM mirror_projects WHERE project_key = ?", [alias])?;
    }
    transaction.execute(
        "INSERT OR REPLACE INTO mirror_projects \
         (project_key, project_name, discord_channel_id, updated_at) VALUES (?, ?, ?, ?)",
        params![canonical_key, project_name, channel_id, now],
    )?;
    transaction.commit()?;
    Ok(aliases)
}

pub fn find_project<F>(
    path: &Path,
    canonical_key: Option<&str>,
    keys_match: F,
) -> Result<Option<ProjectMatch>>
where
    F: Fn(&str, &str) -> bool,
{
    let canonical = canonical_key.unwrap_or_default().trim();
    if canonical.is_empty() {
        return Ok(None);
    }
    let connection = open_initialized(path)?;
    let exact = connection
        .query_row(
            "SELECT discord_channel_id, project_key FROM mirror_projects WHERE project_key = ?",
            [canonical],
            |row| {
                Ok(ProjectMatch {
                    channel_id: row.get(0)?,
                    stored_key: row.get(1)?,
                })
            },
        )
        .optional()?;
    if exact.is_some() {
        return Ok(exact);
    }
    let mut statement = connection.prepare(
        "SELECT discord_channel_id, project_key FROM mirror_projects ORDER BY updated_at DESC",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(ProjectMatch {
                channel_id: row.get(0)?,
                stored_key: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .find(|row| keys_match(&row.stored_key, canonical)))
}

pub fn project_for_channel(
    path: &Path,
    channel_id: Option<i64>,
) -> Result<Option<(String, String)>> {
    let Some(channel_id) = channel_id.filter(|value| *value != 0) else {
        return Ok(None);
    };
    Ok(open_initialized(path)?
        .query_row(
            "SELECT project_key, project_name FROM mirror_projects WHERE discord_channel_id = ?",
            [channel_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

pub fn describe_project_channel(path: &Path, channel_id: Option<i64>) -> Result<String> {
    let Some(channel_id) = channel_id.filter(|value| *value != 0) else {
        return Ok(String::new());
    };
    let connection = open_initialized(path)?;
    let project = connection
        .query_row(
            "SELECT project_name FROM mirror_projects WHERE discord_channel_id = ?",
            [channel_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(project) = project else {
        return Ok(String::new());
    };
    let mut statement = connection.prepare(
        "SELECT thread_title FROM mirror_threads WHERE discord_channel_id = ? \
         ORDER BY updated_at DESC LIMIT 10",
    )?;
    let titles = statement
        .query_map([channel_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
        .collect::<Vec<_>>();
    if titles.len() <= 1 {
        return Ok(String::new());
    }
    let mut lines = vec![
        format!("`{project}` project channel has multiple Codex threads."),
        "Send the message inside one of its Discord threads:".into(),
    ];
    lines.extend(titles.into_iter().map(|title| format!("- {title}")));
    Ok(lines.join("\n"))
}

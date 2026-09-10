//! Read-only snapshot preserving project-to-thread relationships.
use cdr_store::mapping::MirrorTarget;
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub(super) struct InspectionSnapshot {
    pub mappings: Vec<MirrorTarget>,
    pub parents: BTreeSet<i64>,
    pub project_issues: Vec<String>,
    pub project_summary: String,
}

pub(super) fn read_snapshot(path: &Path) -> cdr_store::Result<InspectionSnapshot> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let transaction = connection.unchecked_transaction()?;
    let rows = transaction.prepare(
        "SELECT codex_thread_id,thread_title,discord_channel_id,discord_thread_id,project_key FROM mirror_threads ORDER BY codex_thread_id",
    )?.query_map([], |row| Ok((MirrorTarget {
        codex_thread_id: row.get(0)?, thread_title: row.get(1)?,
        discord_channel_id: row.get(2)?, discord_thread_id: row.get(3)?,
    }, row.get::<_,String>(4)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let projects = transaction
        .prepare("SELECT project_key,discord_channel_id FROM mirror_projects ORDER BY project_key")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    transaction.commit()?;
    let mut channels = BTreeMap::<i64, usize>::new();
    for (_, channel) in &projects {
        *channels.entry(*channel).or_default() += 1;
    }
    let mut issues = Vec::new();
    let mut duplicate = 0;
    for (channel, count) in &channels {
        if *count > 1 {
            duplicate += 1;
            issues.push(format!(
                "duplicate_project_channel | channel={channel} | projects={count}"
            ));
        }
    }
    let (mut missing, mut mismatch, mut ambiguous) = (0, 0, 0);
    for (row, key) in &rows {
        let candidates = projects
            .iter()
            .filter(|(project, _)| super::channels::keys_match(project, key))
            .collect::<Vec<_>>();
        let issue = match candidates.as_slice() {
            [] => {
                missing += 1;
                Some("missing_project_mapping")
            }
            [(_, channel)] if *channel != row.discord_channel_id => {
                mismatch += 1;
                Some("project_parent_mismatch")
            }
            [_] => None,
            _ => {
                ambiguous += 1;
                Some("ambiguous_project_mapping")
            }
        };
        if let Some(issue) = issue {
            issues.push(format!(
                "{issue} | thread={} | stored_parent={}",
                row.codex_thread_id, row.discord_channel_id
            ));
        }
    }
    Ok(InspectionSnapshot {
        mappings: rows.into_iter().map(|(row, _)| row).collect(),
        parents: channels.into_keys().collect(),
        project_issues: issues,
        project_summary: format!(
            "missing_project_mapping: {missing}\nproject_parent_mismatch: {mismatch}\nambiguous_project_mapping: {ambiguous}\nduplicate_project_channels: {duplicate}"
        ),
    })
}

//! Inspection never calls sync or initializes/migrates the mapping database.
use std::collections::{BTreeMap, BTreeSet};

use cdr_codex_state::CodexThreadStore;
use cdr_store::mapping::MirrorTarget;
use twilight_model::channel::ChannelType;

use super::inspection_snapshot::{InspectionSnapshot, read_snapshot};
use super::{MirrorSyncError, MirrorSynchronizer, discord_id};

impl MirrorSynchronizer {
    pub async fn inspect(
        &self,
        origin: u64,
        limit: Option<u32>,
        list: bool,
    ) -> Result<String, MirrorSyncError> {
        let _guard = self.lock.lock().await;
        let InspectionSnapshot {
            mappings,
            mut parents,
            project_issues,
            project_summary,
        } = read_snapshot(&self.mirror_db)?;
        let projects_ok = project_issues.is_empty();
        let store = CodexThreadStore::open(&self.state_db)?;
        let active = store
            .load_recent_threads(0)?
            .into_iter()
            .map(|thread| (thread.id.clone(), thread))
            .collect::<BTreeMap<_, _>>();
        let mut expected = store
            .load_mirror_root_threads(0)?
            .into_iter()
            .map(|thread| thread.id)
            .collect::<BTreeSet<_>>();
        expected.extend(
            mappings
                .iter()
                .filter(|row| active.contains_key(&row.codex_thread_id))
                .map(|row| row.codex_thread_id.clone()),
        );
        let mapped = mappings
            .iter()
            .map(|row| row.codex_thread_id.clone())
            .collect::<BTreeSet<_>>();
        let missing = expected.difference(&mapped).collect::<Vec<_>>();
        let mut rooms = BTreeMap::<i64, usize>::new();
        for row in &mappings {
            *rooms.entry(row.discord_thread_id).or_default() += 1;
            parents.insert(row.discord_channel_id);
        }
        let duplicates = rooms.values().filter(|count| **count > 1).count();
        let stale_ids = mappings
            .iter()
            .filter(|row| !active.contains_key(&row.codex_thread_id))
            .map(|row| row.codex_thread_id.clone())
            .collect::<BTreeSet<_>>();
        let guild = self.inspection_guild(origin).await?;
        let mut details = missing
            .iter()
            .map(|id| missing_mapping(id, &active))
            .collect::<Vec<_>>();
        details.extend(project_issues);
        let missing_rollouts = expected
            .iter()
            .filter_map(|id| active.get(id))
            .filter(|thread| !thread.rollout_path.is_file())
            .collect::<Vec<_>>();
        details.extend(
            missing_rollouts
                .iter()
                .map(|t| format!("missing_rollout | {}", t.id)),
        );
        let mut remote_errors = 0;
        for parent in parents {
            let result = self
                .inspect_channel(parent, guild, ChannelType::GuildText, None)
                .await;
            if result != "ok" {
                remote_errors += 1;
                details.push(format!("project_channel {parent} | {result}"));
            }
        }
        remote_errors += self
            .inspect_rows(&mappings, &stale_ids, &rooms, guild, list, &mut details)
            .await;
        let status = if missing.is_empty()
            && projects_ok
            && duplicates == 0
            && stale_ids.is_empty()
            && missing_rollouts.is_empty()
            && remote_errors == 0
        {
            "ok"
        } else {
            "issues_found"
        };
        let total_details = details.len();
        if let Some(limit) = limit {
            details.truncate(limit as usize);
        }
        Ok(format!(
            "Discord mirror {} (read-only)\nscope: configured local Codex DB; interactive user roots + mapped active threads; writer ownership not verified\nstatus: {status}\nexpected_threads: {}\ntargets: {}\nmissing_mapping: {}\nduplicate_rooms: {duplicates}\nstale_mappings: {}\nmissing_rollouts: {}\nremote_errors: {remote_errors}\n{project_summary}\ndetails: {}/{} (limit affects display only)\n{}",
            if list { "list" } else { "check" },
            expected.len(),
            mappings.len(),
            missing.len(),
            stale_ids.len(),
            missing_rollouts.len(),
            details.len(),
            total_details,
            details.join("\n")
        ))
    }

    async fn inspection_guild(&self, origin: u64) -> Result<u64, MirrorSyncError> {
        match self.guild_id {
            Some(guild) => Ok(guild),
            None => self
                .remote
                .channel(origin)
                .await?
                .and_then(|c| c.guild_id)
                .ok_or_else(|| MirrorSyncError::Invalid("cannot resolve inspection guild".into())),
        }
    }

    async fn inspect_rows(
        &self,
        mappings: &[MirrorTarget],
        stale_ids: &BTreeSet<String>,
        rooms: &BTreeMap<i64, usize>,
        guild: u64,
        list: bool,
        details: &mut Vec<String>,
    ) -> usize {
        let mut remote_errors = 0;
        for row in mappings {
            let result = self
                .inspect_channel(
                    row.discord_thread_id,
                    guild,
                    ChannelType::PublicThread,
                    discord_id(row.discord_channel_id).ok(),
                )
                .await;
            let mut issues = Vec::new();
            if stale_ids.contains(&row.codex_thread_id) {
                issues.push("stale_mapping".to_owned());
            }
            if rooms[&row.discord_thread_id] > 1 {
                issues.push("duplicate_room".to_owned());
            }
            if row.discord_channel_id <= 0 {
                issues.push("invalid_parent".to_owned());
            }
            if result != "ok" {
                remote_errors += 1;
                issues.push(result);
            }
            if list || !issues.is_empty() {
                details.push(format!(
                    "{} | room={} | title={} | {}",
                    row.codex_thread_id,
                    row.discord_thread_id,
                    row.thread_title.replace(['\r', '\n'], " "),
                    if issues.is_empty() {
                        "ok".into()
                    } else {
                        issues.join("; ")
                    }
                ));
            }
        }
        remote_errors
    }

    async fn inspect_channel(
        &self,
        id: i64,
        guild: u64,
        kind: ChannelType,
        parent: Option<u64>,
    ) -> String {
        let id = match discord_id(id) {
            Ok(id) => id,
            Err(error) => return error.to_string(),
        };
        match self.remote.channel(id).await {
            Err(error) => format!("access_error: {error}"),
            Ok(None) => "missing_room".into(),
            Ok(Some(channel)) => match super::channels::validate(&channel, guild, kind, parent) {
                Err(error) => error.to_string(),
                Ok(()) if channel.archived => "discord_room_archived".into(),
                Ok(()) => "ok".into(),
            },
        }
    }
}

fn missing_mapping(id: &str, active: &BTreeMap<String, cdr_codex_state::ThreadInfo>) -> String {
    match active.get(id) {
        Some(thread) => format!(
            "missing_mapping | {id} | title={} | cwd={}",
            thread.title.replace(['\r', '\n'], " "),
            thread.cwd.replace(['\r', '\n'], " "),
        ),
        None => format!("missing_mapping | {id} | source changed during inventory; recheck"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_new_root_between_inventory_reads_is_reported_without_panicking() {
        let text = super::missing_mapping("newly-created", &std::collections::BTreeMap::new());
        assert!(text.contains("missing_mapping | newly-created"));
        assert!(text.contains("source changed during inventory; recheck"));
    }
}

use super::{MirrorSyncError, MirrorSyncResult, MirrorSynchronizer, db_id, names, now};
use cdr_codex_state::load_session_thread_names;
use cdr_store::mapping::{MirrorThreadUpdate, commit_thread_sync};
use std::collections::BTreeMap;

impl MirrorSynchronizer {
    pub async fn sync(
        &self,
        origin_channel: u64,
        limit: Option<i64>,
    ) -> Result<MirrorSyncResult, MirrorSyncError> {
        self.with_operation_deadline("sync", false, self.sync_locked(origin_channel, limit))
            .await
    }

    async fn sync_locked(
        &self,
        origin_channel: u64,
        limit: Option<i64>,
    ) -> Result<MirrorSyncResult, MirrorSyncError> {
        self.ensure_cleanup_reconciled()?;
        let started = now()?;
        let threads = self.scope(limit)?;
        let guild = match self.guild_id {
            Some(guild) => guild,
            None => self
                .remote
                .channel(origin_channel)
                .await?
                .and_then(|channel| channel.guild_id)
                .ok_or_else(|| {
                    MirrorSyncError::Invalid(
                        "cannot resolve the Discord guild for this command".into(),
                    )
                })?,
        };
        let mut channels = self.remote.channels(guild).await?;
        let category = self.category_channel(guild, &channels).await?;
        let index = self
            .state_db
            .parent()
            .ok_or_else(|| MirrorSyncError::Invalid("Codex state has no directory".into()))?
            .join("session_index.jsonl");
        let titles = load_session_thread_names(&index)?;
        let mut projects = BTreeMap::new();
        let mut result = MirrorSyncResult {
            limited: limit.is_some(),
            ..MirrorSyncResult::default()
        };
        for thread in threads {
            if !thread.rollout_path.is_file() {
                result.unavailable += 1;
                eprintln!(
                    "mirror_sync_unavailable thread={} reason=rollout_missing",
                    thread.id
                );
                continue;
            }
            let (key, project_name) = names::project(&thread);
            let parent = if let Some(parent) = projects.get(&key) {
                *parent
            } else {
                let channel = self
                    .project_channel(guild, category, &key, &project_name, &mut channels)
                    .await?;
                projects.insert(key.clone(), channel.id);
                channel.id
            };
            let title =
                names::thread_name(titles.get(&thread.id).unwrap_or(&thread.title), &thread.id);
            let (channel, expected) = self
                .thread_channel(guild, parent, &thread.id, &title)
                .await?;
            commit_thread_sync(
                &self.mirror_db,
                MirrorThreadUpdate {
                    thread_id: &thread.id,
                    project_key: &key,
                    title: &title,
                    parent_id: db_id(parent)?,
                    channel_id: db_id(channel.id)?,
                    now: now()?,
                },
                expected,
            )?;
            result.threads += 1;
        }
        result.projects = projects.len();
        if limit.is_none() {
            result.archived = self.retire_obsolete(guild, started).await?;
            (result.orphan_deleted, result.projects_deleted) =
                self.cleanup_orphans(guild, started).await?;
        }
        Ok(result)
    }
}

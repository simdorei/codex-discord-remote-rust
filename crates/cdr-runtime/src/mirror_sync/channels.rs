use super::{MirrorChannel, MirrorSyncError, MirrorSynchronizer, db_id, discord_id, names, now};
use cdr_codex_state::normalize_workspace_path;
use cdr_store::mapping::{find_project, thread_channels, upsert_project};
use sha2::{Digest, Sha256};
use twilight_model::channel::ChannelType;

impl MirrorSynchronizer {
    pub(super) async fn project_channel(
        &self,
        guild: u64,
        category: u64,
        key: &str,
        name: &str,
        channels: &mut Vec<MirrorChannel>,
    ) -> Result<MirrorChannel, MirrorSyncError> {
        let stored = find_project(&self.mirror_db, Some(key), keys_match)?;
        let mut channel = if let Some(stored) = stored {
            self.remote.channel(discord_id(stored.channel_id)?).await?
        } else {
            None
        };
        let topic = format!("Codex project mirror: {name}");
        let mut expected_name = names::channel_name(name);
        if channel.is_none() {
            for candidate in channels.iter().filter(|channel| {
                channel.kind == ChannelType::GuildText
                    && channel.parent_id == Some(category)
                    && channel.topic.as_deref() == Some(&topic)
            }) {
                if cdr_store::mapping::project_for_channel(
                    &self.mirror_db,
                    Some(db_id(candidate.id)?),
                )?
                .is_none()
                {
                    channel = Some(candidate.clone());
                    break;
                }
            }
        }
        if channels.iter().any(|item| {
            item.name == expected_name
                && channel.as_ref().is_none_or(|current| current.id != item.id)
        }) {
            let hash = hex::encode(Sha256::digest(key.as_bytes()));
            expected_name = format!("{expected_name}-{}", &hash[..6]);
        }
        let channel = if let Some(mut channel) = channel {
            validate(&channel, guild, ChannelType::GuildText, None)?;
            if channel.name != expected_name || channel.topic.as_deref() != Some(&topic) {
                self.remote
                    .update(&channel, &expected_name, Some(&topic), false)
                    .await?;
                channel.name.clone_from(&expected_name);
                channel.topic = Some(topic.clone());
                if let Some(cached) = channels.iter_mut().find(|cached| cached.id == channel.id) {
                    *cached = channel.clone();
                }
            }
            channel
        } else {
            let channel = self
                .remote
                .create(
                    guild,
                    Some(category),
                    ChannelType::GuildText,
                    &expected_name,
                    Some(&topic),
                )
                .await?;
            channels.push(channel.clone());
            channel
        };
        upsert_project(
            &self.mirror_db,
            key,
            name,
            db_id(channel.id)?,
            now()?,
            keys_match,
        )?;
        Ok(channel)
    }

    pub(super) async fn thread_channel(
        &self,
        guild: u64,
        parent: u64,
        thread: &str,
        name: &str,
    ) -> Result<(MirrorChannel, Option<(i64, i64)>), MirrorSyncError> {
        let expected = thread_channels(&self.mirror_db, thread)?;
        let channel = if let Some((_, id)) = expected {
            self.remote.channel(discord_id(id)?).await?
        } else {
            None
        };
        let channel = if let Some(channel) = channel {
            validate(&channel, guild, ChannelType::PublicThread, Some(parent))?;
            if channel.name != name || channel.archived {
                self.remote.update(&channel, name, None, false).await?;
            }
            channel
        } else {
            self.remote
                .create(guild, Some(parent), ChannelType::PublicThread, name, None)
                .await?
        };
        Ok((channel, expected))
    }
}

pub(super) fn keys_match(left: &str, right: &str) -> bool {
    left == right || normalize_workspace_path(left) == normalize_workspace_path(right)
}

pub(super) fn validate(
    channel: &MirrorChannel,
    guild: u64,
    kind: ChannelType,
    parent: Option<u64>,
) -> Result<(), MirrorSyncError> {
    if channel.guild_id != Some(guild)
        || channel.kind != kind
        || parent.is_some_and(|id| channel.parent_id != Some(id))
    {
        return Err(MirrorSyncError::Invalid(format!(
            "stored channel {} has the wrong guild, kind, or parent",
            channel.id
        )));
    }
    Ok(())
}

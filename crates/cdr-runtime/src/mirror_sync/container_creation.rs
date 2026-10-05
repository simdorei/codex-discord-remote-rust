use super::{
    MirrorChannel, MirrorSyncError, MirrorSynchronizer, channels::validate, db_id, discord_id,
};
use cdr_store::mapping::container_creation::{self as custody, ProjectSnapshot};
use twilight_model::channel::ChannelType;

impl MirrorSynchronizer {
    pub(super) async fn category_channel(
        &self,
        guild: u64,
        channels: &[MirrorChannel],
    ) -> Result<u64, MirrorSyncError> {
        let previous = custody::category_receipt(&self.mirror_db, db_id(guild)?)?;
        if let Some(receipt) = &previous {
            let id = discord_id(receipt.confirmed_channel()?)?;
            if let Some(channel) = self.remote.channel(id).await? {
                validate_category(&channel, guild)?;
                if channel.id != id {
                    return Err(MirrorSyncError::Invalid(
                        "category response identity changed".into(),
                    ));
                }
                custody::bind_category(&self.mirror_db, receipt)?;
                return Ok(id);
            }
            if !receipt.is_bound() {
                return Err(MirrorSyncError::Invalid(
                    "confirmed category is missing before binding; creation not repeated".into(),
                ));
            }
            // Only a bound exact ID plus this fresh 404 authorizes a new creation.
        } else {
            let categories = channels
                .iter()
                .filter(|channel| {
                    channel.kind == ChannelType::GuildCategory && channel.name == "Codex"
                })
                .collect::<Vec<_>>();
            if categories.len() > 1 {
                return Err(MirrorSyncError::Invalid(
                    "multiple Codex categories; resolve the duplicate first".into(),
                ));
            }
            if let Some(category) = categories.first() {
                validate_category(category, guild)?;
                return Ok(category.id);
            }
        }
        let receipt = custody::begin_category(&self.mirror_db, db_id(guild)?, previous.as_ref())?;
        let channel = self
            .remote
            .create(guild, None, ChannelType::GuildCategory, "Codex", None)
            .await?;
        validate_category(&channel, guild)?;
        let receipt = custody::confirm(&self.mirror_db, &receipt, db_id(channel.id)?)?;
        custody::bind_category(&self.mirror_db, &receipt)?;
        Ok(channel.id)
    }

    pub(super) async fn confirmed_project_creation(
        &self,
        guild: u64,
        parent: u64,
        key: &str,
        expected: &ProjectSnapshot,
    ) -> Result<Option<MirrorChannel>, MirrorSyncError> {
        let Some(receipt) = custody::project_receipt(
            &self.mirror_db,
            key,
            db_id(guild)?,
            db_id(parent)?,
            expected,
            super::channels::keys_match,
        )?
        else {
            return Ok(None);
        };
        let id = discord_id(receipt.confirmed_channel()?)?;
        let channel = self.remote.channel(id).await?.ok_or_else(|| {
            MirrorSyncError::Invalid(
                "confirmed project room is missing; creation not repeated".into(),
            )
        })?;
        validate(&channel, guild, ChannelType::GuildText, Some(parent))?;
        if channel.id != id {
            return Err(MirrorSyncError::Invalid(
                "project response identity changed".into(),
            ));
        }
        Ok(Some(channel))
    }

    pub(super) async fn create_project_with_receipt(
        &self,
        guild: u64,
        parent: u64,
        key: &str,
        expected: &ProjectSnapshot,
        name: &str,
        topic: &str,
    ) -> Result<MirrorChannel, MirrorSyncError> {
        let receipt = custody::begin_project(
            &self.mirror_db,
            key,
            db_id(guild)?,
            db_id(parent)?,
            expected,
            super::channels::keys_match,
        )?;
        let channel = self
            .remote
            .create(
                guild,
                Some(parent),
                ChannelType::GuildText,
                name,
                Some(topic),
            )
            .await?;
        validate(&channel, guild, ChannelType::GuildText, Some(parent))?;
        custody::confirm(&self.mirror_db, &receipt, db_id(channel.id)?)?;
        Ok(channel)
    }
}

fn validate_category(channel: &MirrorChannel, guild: u64) -> Result<(), MirrorSyncError> {
    validate(channel, guild, ChannelType::GuildCategory, None)?;
    if channel.id == 0 || channel.parent_id.is_some() {
        return Err(MirrorSyncError::Invalid(
            "category response has invalid identity or parent".into(),
        ));
    }
    Ok(())
}

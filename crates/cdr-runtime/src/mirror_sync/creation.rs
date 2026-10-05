use super::{
    MirrorChannel, MirrorSyncError, MirrorSynchronizer, channels::validate, db_id, discord_id,
};
use cdr_store::mapping::creation::{self, CreationScope};
use twilight_model::channel::ChannelType;

impl MirrorSynchronizer {
    pub(super) async fn confirmed_thread_creation(
        &self,
        scope: CreationScope<'_>,
    ) -> Result<Option<MirrorChannel>, MirrorSyncError> {
        let Some(id) = creation::confirmed(&self.mirror_db, scope)? else {
            return Ok(None);
        };
        let id = discord_id(id)?;
        let channel = self.remote.channel(id).await?.ok_or_else(|| {
            MirrorSyncError::Invalid(format!(
                "confirmed created room {id} is unavailable; creation will not be repeated"
            ))
        })?;
        if channel.id != id {
            return Err(MirrorSyncError::Invalid(
                "created room response identity changed".into(),
            ));
        }
        validate(
            &channel,
            discord_id(scope.guild)?,
            ChannelType::PublicThread,
            Some(discord_id(scope.parent)?),
        )?;
        Ok(Some(channel))
    }

    pub(super) async fn create_thread_with_receipt(
        &self,
        scope: CreationScope<'_>,
        name: &str,
    ) -> Result<MirrorChannel, MirrorSyncError> {
        let guild = discord_id(scope.guild)?;
        let parent = discord_id(scope.parent)?;
        let token = creation::begin(&self.mirror_db, scope)?;
        // Cancellation or any ambiguous response retains the durable attempted row.
        let channel = self
            .remote
            .create(guild, Some(parent), ChannelType::PublicThread, name, None)
            .await?;
        validate(&channel, guild, ChannelType::PublicThread, Some(parent))?;
        creation::confirm(&self.mirror_db, scope, &token, db_id(channel.id)?)?;
        Ok(channel)
    }
}

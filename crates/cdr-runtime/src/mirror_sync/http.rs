use super::{MirrorChannel, MirrorFuture, MirrorInventoryThread, MirrorSyncError, MirrorTransport};
use std::sync::Arc;
use twilight_http::{Client, error::ErrorType};
use twilight_model::{
    channel::{Channel, ChannelType, thread::AutoArchiveDuration},
    id::{Id, marker::ChannelMarker},
};

pub struct DiscordMirrorTransport(Arc<Client>);

#[path = "http_cleanup.rs"]
mod cleanup;

impl DiscordMirrorTransport {
    #[must_use]
    pub const fn new(client: Arc<Client>) -> Self {
        Self(client)
    }
}

impl MirrorTransport for DiscordMirrorTransport {
    fn thread_inventory(
        &self,
        guild: u64,
        parent: u64,
    ) -> MirrorFuture<'_, Vec<MirrorInventoryThread>> {
        Box::pin(self.read_thread_inventory(guild, parent))
    }

    fn delete(&self, id: u64) -> MirrorFuture<'_, ()> {
        Box::pin(async move {
            match self.0.delete_channel(channel_id(id)?).await {
                Ok(_) => Ok(()),
                Err(error) if matches!(error.kind(), ErrorType::Response { status, .. } if status.get() == 404) => {
                    Ok(())
                }
                Err(error) if matches!(error.kind(), ErrorType::Response { status, .. } if matches!(status.get(),400|401|403|405|413|415|422|429)) => {
                    Err(MirrorSyncError::DeleteRejected(error.to_string()))
                }
                Err(error) => Err(request_error(error)),
            }
        })
    }
    fn channels(&self, guild: u64) -> MirrorFuture<'_, Vec<MirrorChannel>> {
        Box::pin(async move {
            let guild = Id::new_checked(guild).ok_or_else(|| invalid("zero guild id"))?;
            let channels = self
                .0
                .guild_channels(guild)
                .await
                .map_err(request_error)?
                .models()
                .await
                .map_err(|error| invalid(&error.to_string()))?;
            Ok(channels.into_iter().map(convert).collect())
        })
    }

    fn channel(&self, id: u64) -> MirrorFuture<'_, Option<MirrorChannel>> {
        Box::pin(async move {
            match self.0.channel(channel_id(id)?).await {
                Ok(response) => Ok(Some(convert(
                    response
                        .model()
                        .await
                        .map_err(|error| invalid(&error.to_string()))?,
                ))),
                Err(error) if matches!(error.kind(), ErrorType::Response { status, .. } if status.get() == 404) => {
                    Ok(None)
                }
                Err(error) => Err(request_error(error)),
            }
        })
    }

    fn create<'a>(
        &'a self,
        guild: u64,
        parent: Option<u64>,
        kind: ChannelType,
        name: &'a str,
        topic: Option<&'a str>,
    ) -> MirrorFuture<'a, MirrorChannel> {
        Box::pin(async move {
            let response = if kind == ChannelType::PublicThread {
                let parent = channel_id(parent.ok_or_else(|| invalid("thread has no parent"))?)?;
                self.0
                    .create_thread(parent, name, kind)
                    .auto_archive_duration(AutoArchiveDuration::Week)
                    .await
            } else {
                let guild = Id::new_checked(guild).ok_or_else(|| invalid("zero guild id"))?;
                let mut request = self.0.create_guild_channel(guild, name).kind(kind);
                if let Some(parent) = parent {
                    request = request.parent_id(channel_id(parent)?);
                }
                if let Some(topic) = topic {
                    request = request.topic(topic);
                }
                request.await
            }
            .map_err(request_error)?;
            Ok(convert(
                response
                    .model()
                    .await
                    .map_err(|error| invalid(&error.to_string()))?,
            ))
        })
    }

    fn update<'a>(
        &'a self,
        channel: &'a MirrorChannel,
        name: &'a str,
        topic: Option<&'a str>,
        archived: bool,
    ) -> MirrorFuture<'a, ()> {
        Box::pin(async move {
            if channel.kind.is_thread() {
                self.0
                    .update_thread(channel_id(channel.id)?)
                    .name(name)
                    .archived(archived)
                    .await
                    .map_err(request_error)?;
            } else {
                let mut request = self.0.update_channel(channel_id(channel.id)?).name(name);
                if let Some(topic) = topic {
                    request = request.topic(topic);
                }
                request.await.map_err(request_error)?;
            }
            Ok(())
        })
    }
}

fn convert(channel: Channel) -> MirrorChannel {
    MirrorChannel {
        id: channel.id.get(),
        guild_id: channel.guild_id.map(Id::get),
        parent_id: channel.parent_id.map(Id::get),
        kind: channel.kind,
        name: channel.name.unwrap_or_default(),
        topic: channel.topic,
        archived: channel
            .thread_metadata
            .is_some_and(|metadata| metadata.archived),
    }
}

fn channel_id(id: u64) -> Result<Id<ChannelMarker>, MirrorSyncError> {
    Id::new_checked(id).ok_or_else(|| invalid("zero channel id"))
}

fn invalid(message: &str) -> MirrorSyncError {
    MirrorSyncError::Invalid(message.to_owned())
}

fn request_error(error: twilight_http::Error) -> MirrorSyncError {
    let description = error.to_string();
    let (kind, _) = error.into_parts();
    let message = match kind {
        ErrorType::Response { status, error, .. } => format!("HTTP {}: {error:?}", status.get()),
        _ => description,
    };
    MirrorSyncError::Discord(message)
}

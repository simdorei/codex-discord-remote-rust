use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{MirrorItem, normalized_text_digest};
use crate::completion_worker::{IdempotentChunk, send_recorded_message_chunk};
use cdr_discord::delivery::{DeliveryPolicy, deliver_text_indexed};
use twilight_http::Client;
use twilight_model::id::{Id, marker::ChannelMarker};

#[must_use]
pub fn session_delivery_identity(
    thread_id: &str,
    item: &MirrorItem,
) -> SessionMirrorDeliveryIdentity {
    if item.dedupe_recent_text {
        SessionMirrorDeliveryIdentity::assistant_text(
            &assistant_scope(thread_id, item),
            &normalized_text_digest(&item.text),
        )
    } else {
        SessionMirrorDeliveryIdentity::event(thread_id, &item.digest)
    }
}

pub(crate) fn assistant_scope(thread_id: &str, item: &MirrorItem) -> String {
    let turn = item.turn_id.as_deref().unwrap_or("");
    format!("{}:{thread_id}:{}:{turn}", thread_id.len(), turn.len())
}

#[cfg(test)]
#[path = "sender_http_tests.rs"]
mod tests;

pub const SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN: &str = "session-mirror/assistant-text/v1";
pub const SESSION_MIRROR_EVENT_NONCE_DOMAIN: &str = "session-mirror/event/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMirrorDeliveryIdentity {
    domain: &'static str,
    logical_key: String,
}

impl SessionMirrorDeliveryIdentity {
    #[must_use]
    pub fn assistant_text(thread_id: &str, normalized_text_digest: &[u8; 32]) -> Self {
        Self::new(
            SESSION_MIRROR_ASSISTANT_TEXT_NONCE_DOMAIN,
            thread_id,
            &hex::encode(normalized_text_digest),
        )
    }

    #[must_use]
    pub fn event(thread_id: &str, event_digest: &str) -> Self {
        Self::new(SESSION_MIRROR_EVENT_NONCE_DOMAIN, thread_id, event_digest)
    }

    #[must_use]
    pub const fn domain(&self) -> &'static str {
        self.domain
    }

    #[must_use]
    pub fn logical_key(&self) -> &str {
        &self.logical_key
    }

    fn new(domain: &'static str, thread_id: &str, digest: &str) -> Self {
        Self {
            domain,
            logical_key: format!("{}:{thread_id}:{digest}", thread_id.len()),
        }
    }
}

pub trait SessionMirrorSender: Send + Sync {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

pub struct DiscordSessionMirrorSender {
    http: Arc<Client>,
    mirror_db: std::path::PathBuf,
}

impl DiscordSessionMirrorSender {
    #[must_use]
    pub const fn new(http: Arc<Client>, mirror_db: std::path::PathBuf) -> Self {
        Self { http, mirror_db }
    }
}

impl SessionMirrorSender for DiscordSessionMirrorSender {
    fn send<'a>(
        &'a self,
        channel_id: u64,
        identity: &'a SessionMirrorDeliveryIdentity,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move {
            let channel = Id::<ChannelMarker>::new_checked(channel_id)
                .ok_or_else(|| "Discord channel identifier must be non-zero".to_owned())?;
            deliver_text_indexed(
                text,
                &DeliveryPolicy {
                    retry_delays: Vec::new(),
                    chunk_markers: true,
                },
                |chunk_index, chunk| {
                    let http = Arc::clone(&self.http);
                    let domain = identity.domain();
                    let logical_key = identity.logical_key();
                    async move {
                        send_recorded_message_chunk(
                            &self.mirror_db,
                            &http,
                            channel,
                            &IdempotentChunk {
                                content: chunk,
                                domain,
                                logical_key: logical_key.to_owned(),
                                chunk_index,
                            },
                        )
                        .await
                    }
                },
            )
            .await
            .map_err(|error| format!("{error:?}"))?;
            Ok(())
        })
    }
}

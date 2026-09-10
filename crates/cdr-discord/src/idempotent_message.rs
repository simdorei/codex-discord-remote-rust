//! Discord message creation with server-side nonce deduplication.
//!
//! Discord documents nonce enforcement as applying to messages sent by the
//! same sender in the preceding few minutes. This closes immediate retry and
//! local-commit crash windows; it is not permanent exactly-once delivery.

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use twilight_http::{Client, request::Request, routing::Route};
use twilight_model::{
    channel::message::{AllowedMentions, Component, Message},
    id::{Id, marker::ChannelMarker},
};

use crate::text::DISCORD_MAX_LEN;

const NONCE_CONTEXT: &[u8] = b"cdr-discord/idempotent-message/v1";
const DISCORD_NONCE_MAX: u64 = i64::MAX as u64;

#[derive(Debug, Error)]
pub enum IdempotentMessageError {
    #[error("Discord message content must not be empty")]
    ContentEmpty,
    #[error("Discord message content has {actual} characters; maximum is {maximum}")]
    ContentTooLong { actual: usize, maximum: usize },
    #[error("Discord HTTP request failed: {0}")]
    Request(#[from] twilight_http::Error),
    #[error("Discord message receipt decode failed: {0}")]
    Receipt(#[from] twilight_http::response::DeserializeBodyError),
}

#[derive(Serialize)]
struct IdempotentMessagePayload<'a> {
    content: &'a str,
    allowed_mentions: AllowedMentions,
    #[serde(skip_serializing_if = "Option::is_none")]
    components: Option<&'a [Component]>,
    nonce: u64,
    enforce_nonce: bool,
}

/// Derive a stable Discord nonce from one logical message-chunk identity.
#[must_use]
pub fn message_nonce(
    domain: &str,
    channel_id: Id<ChannelMarker>,
    logical_key: &str,
    chunk_index: usize,
) -> u64 {
    let mut digest = Sha256::new();
    digest.update(NONCE_CONTEXT);
    update_length_prefixed(&mut digest, domain.as_bytes());
    digest.update(channel_id.get().to_be_bytes());
    update_length_prefixed(&mut digest, logical_key.as_bytes());
    digest.update(
        u64::try_from(chunk_index)
            .expect("Rust usize values fit in u64 on supported targets")
            .to_be_bytes(),
    );
    let bytes: [u8; 32] = digest.finalize().into();
    u64::from_be_bytes(
        bytes[..8]
            .try_into()
            .expect("SHA-256 has eight prefix bytes"),
    ) & DISCORD_NONCE_MAX
}

/// Build the exact JSON request used for nonce-enforced message creation.
pub fn idempotent_message_request(
    channel_id: Id<ChannelMarker>,
    content: &str,
    domain: &str,
    logical_key: &str,
    chunk_index: usize,
) -> Result<Request, IdempotentMessageError> {
    idempotent_message_request_with_components(
        channel_id,
        content,
        &[],
        domain,
        logical_key,
        chunk_index,
    )
}

/// Build a nonce-enforced message request while preserving Discord components.
pub fn idempotent_message_request_with_components(
    channel_id: Id<ChannelMarker>,
    content: &str,
    components: &[Component],
    domain: &str,
    logical_key: &str,
    chunk_index: usize,
) -> Result<Request, IdempotentMessageError> {
    if content.trim().is_empty() {
        return Err(IdempotentMessageError::ContentEmpty);
    }
    let actual = content.chars().count();
    if actual > DISCORD_MAX_LEN {
        return Err(IdempotentMessageError::ContentTooLong {
            actual,
            maximum: DISCORD_MAX_LEN,
        });
    }
    let payload = IdempotentMessagePayload {
        content,
        allowed_mentions: AllowedMentions::default(),
        components: (!components.is_empty()).then_some(components),
        nonce: message_nonce(domain, channel_id, logical_key, chunk_index),
        enforce_nonce: true,
    };
    Ok(Request::builder(&Route::CreateMessage {
        channel_id: channel_id.get(),
    })
    .json(&payload)
    .build()?)
}

/// Send one message chunk through Discord's nonce-enforced create endpoint.
pub async fn send_idempotent_message(
    client: &Client,
    channel_id: Id<ChannelMarker>,
    content: &str,
    domain: &str,
    logical_key: &str,
    chunk_index: usize,
) -> Result<(), IdempotentMessageError> {
    send_idempotent_message_with_components(
        client,
        channel_id,
        content,
        &[],
        domain,
        logical_key,
        chunk_index,
    )
    .await
}

/// Send one nonce-enforced message chunk with optional Discord components.
pub async fn send_idempotent_message_with_components(
    client: &Client,
    channel_id: Id<ChannelMarker>,
    content: &str,
    components: &[Component],
    domain: &str,
    logical_key: &str,
    chunk_index: usize,
) -> Result<(), IdempotentMessageError> {
    let request = idempotent_message_request_with_components(
        channel_id,
        content,
        components,
        domain,
        logical_key,
        chunk_index,
    )?;
    client.request::<Message>(request).await?;
    Ok(())
}

fn update_length_prefixed(digest: &mut Sha256, value: &[u8]) {
    digest.update(
        u64::try_from(value.len())
            .expect("Rust usize values fit in u64 on supported targets")
            .to_be_bytes(),
    );
    digest.update(value);
}

/// Preserve the server-confirmed identity for durable, cross-restart deduplication.
pub async fn send_idempotent_message_receipt(
    client: &Client,
    channel_id: Id<ChannelMarker>,
    content: &str,
    domain: &str,
    logical_key: &str,
    chunk_index: usize,
) -> Result<String, IdempotentMessageError> {
    let request =
        idempotent_message_request(channel_id, content, domain, logical_key, chunk_index)?;
    Ok(client
        .request::<Message>(request)
        .await?
        .model()
        .await?
        .id
        .to_string())
}

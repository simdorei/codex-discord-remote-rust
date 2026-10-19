//! Use Discord's own nonce echo as a receipt, never as permission to resend.
use std::path::Path;

use cdr_discord::{idempotent_message::message_nonce, text::split_delivery_chunks};
use cdr_store::{Result, delivery_receipt, ingress};
use sha2::{Digest, Sha256};
use twilight_model::channel::Message;

pub(super) fn confirm(
    db: &Path,
    message: &Message,
    nonce: Option<u64>,
    bot_user_id: Option<u64>,
) -> Result<bool> {
    let Some(nonce) = nonce else {
        return Ok(false);
    };
    if Some(message.author.id.get()) != bot_user_id
        || !message.author.bot
        || message.webhook_id.is_some()
        || message.edited_timestamp.is_some()
    {
        return Ok(false);
    }
    let hash = if message.components.is_empty() {
        hex::encode(Sha256::digest(message.content.as_bytes()))
    } else {
        hex::encode(Sha256::digest(serde_json::to_vec(&(
            &message.content,
            &message.components,
        ))?))
    };
    let mut matched = None;
    for key in delivery_receipt::candidates_for_hash(db, &hash, &message.id.to_string())? {
        let Ok((channel, domain, logical, part)) =
            serde_json::from_str::<(u64, String, String, usize)>(&key)
        else {
            continue;
        };
        if channel == message.channel_id.get()
            && message_nonce(&domain, message.channel_id, &logical, part) == nonce
        {
            if matched.is_some() {
                return Ok(false);
            }
            matched = Some((key, domain, logical));
        }
    }
    let Some((key, domain, logical)) = matched else {
        return Ok(false);
    };
    if !delivery_receipt::confirm(db, &key, &message.id.to_string())? {
        return Ok(false);
    }
    if domain == "message/reply/v1"
        && let Some(event) = logical
            .strip_prefix("inbound-message/")
            .and_then(|s| s.strip_suffix("/action-result"))
            .and_then(|s| s.parse::<u64>().ok())
        && let Some(record) = ingress::get(db, &format!("message:{event}"))?
        && let Some(response) = record
            .outcome
            .as_ref()
            .and_then(|value| value.get("response"))
            .and_then(|value| value.as_str())
    {
        delivery_receipt::confirm_legacy_message_reply(
            db,
            message.channel_id.get(),
            event,
            response,
            &split_delivery_chunks(response, true),
        )?;
    }
    eprintln!(
        "discord_gateway_receipt_confirmed channel_id={} message_id={}",
        message.channel_id, message.id
    );
    Ok(true)
}

#[cfg(test)]
mod tests;

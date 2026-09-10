use super::{NewReply, acknowledgement_key, get_in};
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn acknowledgement_sendable(path: &Path, record: &NewReply) -> Result<bool> {
    if !record.acknowledgement_recovery_allowed {
        return Ok(false);
    }
    let key = acknowledgement_key(record)?;
    let state: Option<(Option<String>,bool,Option<String>)>=open_initialized(path)?.query_row(
        "SELECT message_id,retryable,blocked_reason FROM codex_delivery_receipts WHERE receipt_key=?",
        [key],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    Ok(state.is_none_or(|(message, retryable, blocked)| {
        message.is_some() || (retryable && blocked.is_none())
    }))
}

pub fn release_acknowledgement(path: &Path, ingress: &str) -> Result<()> {
    open_initialized(path)?.execute("UPDATE codex_new_first_replies SET ack_recovery_allowed=1 WHERE ingress_id=? AND confirmation_delivered=0",[ingress])?;
    Ok(())
}

pub(super) fn validate_notice_in(connection: &Connection, key: &str, hash: &str) -> Result<()> {
    let Ok((channel, domain, job, part)) =
        serde_json::from_str::<(i64, String, String, usize)>(key)
    else {
        return Ok(());
    };
    if domain != "new/verification-notice/v1" {
        return Ok(());
    }
    let record = get_in(connection, &job)?
        .ok_or_else(|| StoreError::Integrity("new warning has no original intent".into()))?;
    let owner: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM discord_ingress_journal WHERE ingress_id=? AND owner_id=? AND channel_id=?)",
        params![record.identity.ingress_id,job,channel],|row|row.get(0))?;
    if !owner
        || channel != record.identity.origin_channel_id
        || part != 0
        || record.warning_due == 0
        || hex::encode(Sha256::digest(super::warning_text(&record).as_bytes())) != hash
    {
        return Err(StoreError::Integrity(
            "new warning identity changed; no notice sent".into(),
        ));
    }
    Ok(())
}

pub(crate) fn notice_claimed_in(connection: &Connection, key: &str) -> Result<()> {
    if let Ok((_, domain, job, _)) = serde_json::from_str::<(i64, String, String, usize)>(key)
        && domain == "new/verification-notice/v1"
    {
        connection.execute(
            "UPDATE codex_new_first_replies SET warning_due=2 WHERE job_id=?",
            [job],
        )?;
    }
    Ok(())
}

pub(crate) fn notice_rejected_in(connection: &Connection, key: &str) -> Result<()> {
    if let Ok((_, domain, job, _)) = serde_json::from_str::<(i64, String, String, usize)>(key)
        && domain == "new/verification-notice/v1"
    {
        connection.execute(
            "UPDATE codex_new_first_replies SET warning_due=1 WHERE job_id=? AND warning_due=2",
            [job],
        )?;
    }
    Ok(())
}

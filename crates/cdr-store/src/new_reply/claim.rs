//! Final send authorization runs under the very same writer lock as receipt claim.
use super::{NewReply, get_in};
use crate::{Result, StoreError, schema::open_initialized};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct DeliveryGuard<'a> {
    pub job_id: &'a str,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

pub fn acknowledgement_key(record: &NewReply) -> Result<String> {
    let event = record
        .identity
        .event_id
        .ok_or_else(|| StoreError::Integrity("new acknowledgement has no event identity".into()))?;
    let (domain, key) = match record.identity.kind {
        crate::ingress::IngressKind::Message => (
            "message/reply/v1",
            format!("inbound-message/{event}/action-result"),
        ),
        crate::ingress::IngressKind::Interaction => {
            ("interaction/initial/v1", format!("interaction/{event}"))
        }
        crate::ingress::IngressKind::Action => {
            return Err(StoreError::Integrity(
                "headless new has no Discord acknowledgement".into(),
            ));
        }
    };
    Ok(serde_json::to_string(&(
        record.identity.origin_channel_id,
        domain,
        key,
        0,
    ))?)
}

pub(crate) fn validate_identity_in(connection: &Connection, record: &NewReply) -> Result<()> {
    let id = &record.identity;
    let matches: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM discord_ingress_journal
        WHERE ingress_id=? AND owner_kind='prompt' AND owner_id=? AND target_thread_id=? AND channel_id=?
        AND json_extract(outcome_json,'$.new_creation.version')=1
        AND json_extract(outcome_json,'$.new_creation.cwd')=?
        AND json_extract(outcome_json,'$.new_verification.thread_id')=?
        AND json_extract(outcome_json,'$.new_verification.channel_id')=?
        AND json_extract(outcome_json,'$.new_verification.prompt_sha256')=? AND phase<>'cancelled')",
        params![id.ingress_id,id.job_id,id.thread_id,id.origin_channel_id,id.cwd,id.thread_id,id.channel_id,id.prompt_sha256],|row|row.get(0))?;
    let mapping = crate::mapping::mirrored_thread_id_in(connection, Some(id.channel_id))?;
    if !matches || mapping.as_deref() != Some(&id.thread_id) {
        return Err(StoreError::Integrity(
            "new first-reply evidence or original room mapping changed; no message sent".into(),
        ));
    }
    Ok(())
}

pub fn output_hold(path: &Path, job: &str) -> Result<Option<String>> {
    let connection = open_initialized(path)?;
    get_in(&connection, job)?
        .map(|record| {
            validate_identity_in(&connection, &record)?;
            Ok(readiness_hold(&record))
        })
        .transpose()
        .map(Option::flatten)
}

fn readiness_hold(record: &NewReply) -> Option<String> {
    if record.state != "verified" {
        Some(format!(
            "new first input verification is {}; output remains saved: {}",
            record.state, record.last_error
        ))
    } else if !record.confirmation_delivered
        && record.identity.kind != crate::ingress::IngressKind::Action
    {
        Some("new first reply is not confirmed; output remains saved".into())
    } else {
        None
    }
}

fn acknowledgement_for_key(connection: &Connection, key: &str) -> Result<Option<NewReply>> {
    let Ok((channel, domain, logical, part)) =
        serde_json::from_str::<(i64, String, String, usize)>(key)
    else {
        return Ok(None);
    };
    if part != 0
        || !matches!(
            domain.as_str(),
            "message/reply/v1" | "interaction/initial/v1"
        )
    {
        return Ok(None);
    }
    let event = if domain == "message/reply/v1" {
        logical
            .strip_prefix("inbound-message/")
            .and_then(|s| s.strip_suffix("/action-result"))
    } else {
        logical.strip_prefix("interaction/")
    }
    .and_then(|v| v.parse::<i64>().ok());
    let Some(event) = event else {
        return Ok(None);
    };
    let job: Option<String> = connection.query_row("SELECT job_id FROM codex_new_first_replies
        WHERE json_extract(identity_json,'$.origin_channel_id')=? AND json_extract(identity_json,'$.event_id')=?",
        params![channel,event],|row|row.get(0)).optional()?;
    job.map(|job| get_in(connection, &job))
        .transpose()
        .map(Option::flatten)
}

pub(crate) fn validate_claim_in(
    connection: &Connection,
    key: &str,
    hash: &str,
    guard: Option<&DeliveryGuard<'_>>,
) -> Result<Option<String>> {
    super::notice::validate_notice_in(connection, key, hash)?;
    if let Some(guard) = guard
        && let Some(record) = get_in(connection, guard.job_id)?
    {
        validate_identity_in(connection, &record)?;
        let (channel, _, _, _): (i64, String, String, usize) = serde_json::from_str(key)?;
        if record.identity.thread_id != guard.thread_id
            || record.identity.channel_id != channel
            || guard.turn_id.is_empty()
        {
            return Err(StoreError::Integrity(
                "new output claim changed its original destination".into(),
            ));
        }
        if record.turn_id.as_deref() != Some(guard.turn_id) {
            let continuation: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM codex_delivery_outbox
                WHERE job_id=?1 AND target_thread_id=?2 AND turn_id=?3 AND channel_id=?4)
                OR EXISTS(SELECT 1 FROM codex_goal_progress WHERE job_id=?1 AND thread=?2 AND turn=?3 AND channel=?4)
                OR EXISTS(SELECT 1 FROM codex_commentary_outbox WHERE job_id=?1 AND target_thread_id=?2 AND turn_id=?3 AND channel_id=?4)",
                params![guard.job_id,guard.thread_id,guard.turn_id,channel],|row|row.get(0))?;
            if !continuation {
                return Err(StoreError::Integrity(
                    "new output claim has no exact-turn ownership evidence".into(),
                ));
            }
        }
        if let Some(hold) = readiness_hold(&record) {
            return Ok(Some(hold));
        }
    }
    if let Some(record) = acknowledgement_for_key(connection, key)? {
        validate_identity_in(connection, &record)?;
        if acknowledgement_key(&record)? != key
            || hex::encode(Sha256::digest(record.identity.acknowledgement.as_bytes())) != hash
        {
            return Err(StoreError::Integrity(
                "new acknowledgement identity/body changed; no POST attempted".into(),
            ));
        }
        if record.turn_id.is_none() {
            return Ok(Some(
                "new turn acceptance is uncertain; normal acknowledgement is not authorized".into(),
            ));
        }
    }
    Ok(None)
}

/// A warning or error receipt can never open the generated-output barrier.
pub(crate) fn confirm_receipt_in(connection: &Connection, key: &str) -> Result<()> {
    let Some(record) = acknowledgement_for_key(connection, key)? else {
        return Ok(());
    };
    validate_identity_in(connection, &record)?;
    let hash = hex::encode(Sha256::digest(record.identity.acknowledgement.as_bytes()));
    let confirmed: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM codex_delivery_receipts WHERE receipt_key=? AND content_hash=? AND message_id IS NOT NULL)",params![key,hash],|row|row.get(0))?;
    if !confirmed {
        return Err(StoreError::Integrity(
            "new acknowledgement receipt is not confirmed".into(),
        ));
    }
    connection.execute(
        "UPDATE codex_new_first_replies SET confirmation_delivered=1 WHERE job_id=?",
        [&record.identity.job_id],
    )?;
    connection.execute("UPDATE discord_ingress_journal SET confirmation_delivered=1 WHERE ingress_id=? AND owner_id=?",params![record.identity.ingress_id,record.identity.job_id])?;
    Ok(())
}

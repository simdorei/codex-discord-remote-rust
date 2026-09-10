use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{IngressAdmission, IngressKind, NewIngress, read};
use crate::{Result, StoreError};

pub fn admit(path: &Path, request: &NewIngress) -> Result<IngressAdmission> {
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let admitted = admit_in(&transaction, request)?;
    transaction.commit()?;
    Ok(admitted)
}

pub(super) fn admit_in(connection: &Connection, request: &NewIngress) -> Result<IngressAdmission> {
    validate(request)?;
    let existing = read::get_in(connection, &request.ingress_id)?.or(request
        .event_id
        .map(|id| read::by_origin_in(connection, id))
        .transpose()?
        .flatten());
    if let Some(existing) = existing {
        if existing.channel_id != request.channel_id
            || existing.owner_user_id != request.owner_user_id
            || existing.kind != request.kind
            || existing.event_id != request.event_id
        {
            return Err(StoreError::Integrity(
                "ingress identity conflicts with its original owner".into(),
            ));
        }
        return Ok(IngressAdmission {
            created: false,
            canonical_repeat_created: false,
            record: Some(existing),
            busy_choice: None,
        });
    }
    if request.kind == IngressKind::Message {
        let already_claimed: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM discord_processed_messages WHERE message_id = ?)",
            [request.event_id],
            |row| row.get(0),
        )?;
        if already_claimed {
            return Ok(IngressAdmission {
                created: false,
                canonical_repeat_created: false,
                record: None,
                busy_choice: None,
            });
        }
    }
    let mut original = request.clone();
    // Production messages carry the classification snapshot. Other new entry
    // points acquire theirs here, at durable admission, never at execution.
    if original.payload.get("new_origin").is_none()
        && (original.payload["command"] == "new"
            || original.payload.pointer("/plan/Execute/New").is_some()
            || original
                .payload
                .pointer("/work/Slash/name")
                .is_some_and(|name| name == "new"))
    {
        original.payload["new_origin"] = serde_json::to_value(
            crate::mapping::new_thread_origin_in(connection, original.channel_id)?,
        )?;
    }
    let request = super::new_prompt_arm::prepare_in(connection, &original)?;
    let runtime_id: Option<String> = connection
        .query_row(
            "SELECT runtime_id FROM codex_app_server_runtime WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    connection.execute(
        "INSERT INTO discord_ingress_journal
         (ingress_id,kind,event_id,application_id,channel_id,owner_user_id,source_message_id,
          payload_json,runtime_id,state,phase,target_thread_id,canonical_owner,created_at,updated_at)
         VALUES (?,?,?,?,?,?,?,?,?,'staged','staged',?,?,?,?)",
        params![request.ingress_id,request.kind.as_str(),request.event_id,request.application_id,
            request.channel_id,request.owner_user_id,request.source_message_id,request.payload.to_string(),
            runtime_id,request.target_thread_id,request.canonical_owner,request.now,request.now],
    )?;
    if request.kind == IngressKind::Message {
        connection.execute(
            "INSERT INTO discord_processed_messages (message_id,seen_at) VALUES (?,?)",
            params![request.event_id, request.now],
        )?;
    }
    Ok(IngressAdmission {
        created: true,
        canonical_repeat_created: false,
        record: read::get_in(connection, &request.ingress_id)?,
        busy_choice: None,
    })
}

fn validate(request: &NewIngress) -> Result<()> {
    if request.ingress_id.trim().is_empty()
        || request.ingress_id.trim() != request.ingress_id
        || request.channel_id <= 0
        || request.owner_user_id <= 0
        || !request.now.is_finite()
        || request.now < 0.0
        || request.event_id.is_some_and(|id| id <= 0)
        || request.kind != IngressKind::Action && request.event_id.is_none()
        || !request.payload.is_object()
    {
        return Err(StoreError::Integrity(
            "invalid ingress custody identity".into(),
        ));
    }
    Ok(())
}

use rusqlite::{Connection, params};

use super::read::{by_origin_in, get_in};
use crate::claims::BusyChoice;
use crate::prompt_intake::StoredPromptIntake;
use crate::{Result, StoreError};

pub(crate) fn verify_new_prompt(
    connection: &Connection,
    key: &str,
    request: &crate::prompt_intake::NewPromptIntake<'_>,
) -> Result<()> {
    let ingress = get_in(connection, key)?
        .ok_or_else(|| StoreError::Integrity(format!("missing new ingress: {key}")))?;
    if super::new_execution_prompt(&ingress)? != Some(request.raw_prompt)
        || ingress.event_id != request.discord_message_id
        || Some(ingress.owner_user_id) != request.owner_user_id
    {
        return Err(StoreError::Integrity(format!(
            "new prompt identity changed: {key}"
        )));
    }
    Ok(())
}

pub(crate) fn link_prompt_owner(
    connection: &Connection,
    intake: &StoredPromptIntake,
) -> Result<()> {
    if let Some(event_id) = intake.discord_message_id
        && let Some(ingress) = by_origin_in(connection, event_id)?
    {
        link_prompt_owner_by_key(connection, &ingress.ingress_id, intake)?;
    }
    Ok(())
}

pub(crate) fn link_prompt_owner_by_key(
    connection: &Connection,
    key: &str,
    intake: &StoredPromptIntake,
) -> Result<()> {
    if connection.is_autocommit() {
        return Err(StoreError::ActiveTransaction);
    }
    let ingress = get_in(connection, key)?
        .ok_or_else(|| StoreError::Integrity(format!("missing ingress handoff: {key}")))?;
    let new_room = super::new_execution_prompt(&ingress)? == Some(intake.raw_prompt.as_str())
        && ingress.target_thread_id.as_deref() == Some(intake.target_thread_id.as_str())
        && matches!(ingress.phase.as_str(), "thread/created" | "durable_prompt")
        && connection.query_row("SELECT EXISTS(SELECT 1 FROM mirror_threads WHERE codex_thread_id=? AND discord_thread_id=?)",
            params![intake.target_thread_id, intake.channel_id], |row| row.get::<_, bool>(0))?;
    if (ingress.channel_id != intake.channel_id && !new_room)
        || super::frozen_slash_target(&ingress)
            .is_some_and(|original| original != intake.target_thread_id)
        || Some(ingress.owner_user_id) != intake.owner_user_id
        || ingress.state == "held"
        || ingress
            .owner_id
            .as_ref()
            .is_some_and(|owner| owner != &intake.job_id)
    {
        return Err(StoreError::Integrity(format!(
            "ingress handoff identity changed: {key}"
        )));
    }
    connection.execute(
        "UPDATE discord_ingress_journal SET state='owned',phase='durable_prompt',owner_kind='prompt',owner_id=?,target_thread_id=?,updated_at=? WHERE ingress_id=?",
        params![intake.job_id,intake.target_thread_id,intake.updated_at,key],
    )?;
    Ok(())
}

pub(crate) fn record_busy_owner(
    connection: &Connection,
    choice: &BusyChoice,
    job_id: &str,
    target: &str,
    now: f64,
) -> Result<()> {
    if connection.is_autocommit() {
        return Err(StoreError::ActiveTransaction);
    }
    let canonical = format!("busy-choice:{}", choice.choice_id);
    connection.execute(
        "INSERT INTO discord_ingress_owner_receipts (owner_key,owner_kind,owner_id,target_thread_id,channel_id,owner_user_id,payload_json,created_at)
         VALUES (?,'prompt',?,?,?,?,?,?) ON CONFLICT(owner_key) DO NOTHING",
        params![canonical,job_id,target,choice.channel_id,choice.owner_user_id,serde_json::to_string(choice)?,now],
    )?;
    connection.execute(
        "UPDATE discord_ingress_journal SET state='owned',phase='durable_prompt',owner_kind='prompt',owner_id=?,target_thread_id=?,updated_at=?
         WHERE canonical_owner=? AND channel_id=? AND owner_user_id=?",
        params![job_id,target,now,canonical,choice.channel_id,choice.owner_user_id],
    )?;
    Ok(())
}

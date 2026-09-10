use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{IngressAdmission, NewIngress, admission::admit_in, read::get_in};
use crate::claims::BusyChoice;
use crate::{Result, StoreError};

/// Freeze button authorization and its original prompt before ACK. The canonical
/// receipt also covers later interaction IDs after temporary choices/jobs expire.
pub fn admit_busy_interaction(
    path: &Path,
    request: &NewIngress,
    choice_id: &str,
    action: &str,
) -> Result<IngressAdmission> {
    if !matches!(action, "queue" | "steer" | "stop" | "ignore") {
        return Err(StoreError::BusyChoiceUnavailable(choice_id.into()));
    }
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let canonical = format!("busy-choice:{choice_id}");
    let receipt: Option<(String, String)> = transaction
        .query_row(
            "SELECT owner_id,payload_json FROM discord_ingress_owner_receipts WHERE owner_key=?",
            [&canonical],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let parent: Option<String> = transaction
        .query_row(
            "SELECT ingress_id FROM discord_ingress_journal WHERE canonical_owner=?
         AND phase != 'canonical_duplicate'
         AND COALESCE(state='completed' AND owner_id IS NULL
             AND json_extract(outcome_json,'$.kind')='busy_control_preflight_rejected'
             AND json_type(outcome_json,'$.control_dispatched')='false',0)=0
         ORDER BY created_at,ingress_id LIMIT 1",
            [&canonical],
            |row| row.get(0),
        )
        .optional()?;
    let choice = if let Some((_, payload)) = &receipt {
        serde_json::from_str::<BusyChoice>(payload)?
    } else if let Some(parent) = &parent {
        let original = get_in(&transaction, parent)?.ok_or_else(|| missing(choice_id))?;
        serde_json::from_value::<BusyChoice>(original.payload["busy_choice"].clone())?
    } else {
        active_choice(&transaction, choice_id, request.now)?.ok_or_else(|| missing(choice_id))?
    };
    // allow_steer is a display snapshot ('Steer now' versus 'Steer (check)'),
    // not authorization. The worker verifies the immutable original turn before RPC.
    if choice.owner_user_id != request.owner_user_id || choice.channel_id != request.channel_id {
        return Err(missing(choice_id));
    }
    let mut frozen = request.clone();
    frozen.canonical_owner = Some(canonical.clone());
    frozen.target_thread_id.clone_from(&choice.target_thread_id);
    frozen.payload["busy_choice"] = serde_json::to_value(&choice)?;
    frozen.payload["busy_action"] = action.into();
    let mut admission = admit_in(&transaction, &frozen)?;
    if admission.created
        && admission
            .record
            .as_ref()
            .is_some_and(|row| row.state != "held")
        && (receipt.is_some() || parent.is_some())
    {
        let (owner_kind, owner_id) = if let Some((job_id, _)) = receipt {
            ("prompt", job_id)
        } else {
            ("ingress", parent.expect("parent checked above"))
        };
        transaction.execute(
            "UPDATE discord_ingress_journal SET state='owned',phase='canonical_duplicate',owner_kind=?,owner_id=? WHERE ingress_id=?",
            params![owner_kind,owner_id,request.ingress_id],
        )?;
        admission.created = false;
        admission.canonical_repeat_created = true;
        admission.record = get_in(&transaction, &request.ingress_id)?;
    }
    admission.busy_choice = Some(choice);
    transaction.commit()?;
    Ok(admission)
}

fn active_choice(connection: &Connection, key: &str, now: f64) -> Result<Option<BusyChoice>> {
    Ok(connection.query_row(
        "SELECT choice_id,owner_user_id,channel_id,target_thread_id,prompt,allow_steer,created_at,expires_at
         FROM busy_choices WHERE choice_id=? AND expires_at>? AND claimed_at IS NULL",
        params![key,now], |row| Ok(BusyChoice {
            choice_id:row.get(0)?,owner_user_id:row.get(1)?,channel_id:row.get(2)?,target_thread_id:row.get(3)?,
            prompt:row.get(4)?,allow_steer:row.get(5)?,created_at:row.get(6)?,expires_at:row.get(7)?,
        }),
    ).optional()?)
}

fn missing(key: &str) -> StoreError {
    StoreError::BusyChoiceUnavailable(key.into())
}

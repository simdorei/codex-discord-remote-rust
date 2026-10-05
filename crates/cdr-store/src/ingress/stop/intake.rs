//! Original preparing requests share the permanent per-request stop hold.
use rusqlite::Connection;
use serde_json::json;

use super::{REASON, StopScope, hold_snapshot, refused};
use crate::{Result, prompt_intake::StoredPromptIntake};

pub(super) fn snapshot(
    db: &Connection,
    scope: StopScope<'_>,
    queued: usize,
) -> Result<Vec<StoredPromptIntake>> {
    let ids = target_ids(db, scope.target)?;
    if queued.saturating_add(ids.len()) > 128 {
        return Err(refused());
    }
    ids.iter()
        .map(|id| {
            let intake = crate::prompt_intake::get_in(db, id)?.ok_or_else(refused)?;
            if intake.channel_id != scope.channel
                || intake.owner_user_id != Some(scope.owner)
                || intake.target_thread_id != scope.target
            {
                return Err(refused());
            }
            Ok(intake)
        })
        .collect()
}

pub(super) fn hold(
    db: &Connection,
    scope: StopScope<'_>,
    originals: &[StoredPromptIntake],
    operation: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    originals
        .iter()
        .map(|intake| {
            let wanted = hold_snapshot(db, &intake.job_id)?.unwrap_or_else(|| {
                (
                    scope.target.to_owned(),
                    REASON.to_owned(),
                    json!({"kind":"stop_preparing","operation_id":operation,
                "job_id":intake.job_id,"target_thread_id":intake.target_thread_id,
                "channel_id":intake.channel_id,"owner_user_id":intake.owner_user_id,
                "discord_message_id":intake.discord_message_id,"claim_token":intake.claim_token})
                    .to_string(),
                )
            });
            if wanted.0 != scope.target {
                return Err(refused());
            }
            crate::execution_hold::hold_in(db, &intake.job_id, scope.target, &wanted.1, &wanted.2)?;
            Ok(wanted)
        })
        .collect()
}

pub(super) fn verify(
    db: &Connection,
    scope: StopScope<'_>,
    originals: &[StoredPromptIntake],
    holds: &[(String, String, String)],
) -> Result<()> {
    let ids: Vec<_> = originals
        .iter()
        .map(|intake| intake.job_id.as_str())
        .collect();
    if target_ids(db, scope.target)? != ids || holds.len() != originals.len() {
        return Err(refused());
    }
    for (intake, hold) in originals.iter().zip(holds) {
        if crate::prompt_intake::get_in(db, &intake.job_id)?.as_ref() != Some(intake)
            || hold_snapshot(db, &intake.job_id)?.as_ref() != Some(hold)
        {
            return Err(refused());
        }
    }
    Ok(())
}

pub(super) fn receipt_ids(queued: &[String], originals: &[StoredPromptIntake]) -> Vec<String> {
    let mut ids = queued.to_vec();
    ids.extend(originals.iter().map(|intake| intake.job_id.clone()));
    ids.sort();
    ids.dedup();
    ids
}

fn target_ids(db: &Connection, target: &str) -> Result<Vec<String>> {
    Ok(db.prepare(
        "SELECT job_id FROM codex_prompt_intakes WHERE target_thread_id=? ORDER BY job_id LIMIT 129",
    )?.query_map([target], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
}

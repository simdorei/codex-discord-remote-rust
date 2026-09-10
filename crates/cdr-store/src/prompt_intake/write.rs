use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, TransactionBehavior, params};

use super::{NewPromptIntake, PromptIntakeAdmission, StoredPromptIntake, storage};
use crate::schema::open_initialized;
use crate::{Result, StoreError};

pub fn admit_prompt_intake(
    path: &Path,
    new_intake: NewPromptIntake<'_>,
) -> Result<PromptIntakeAdmission> {
    validate_identity(new_intake.job_id, new_intake.target_thread_id)?;
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let admitted = admit_in_transaction(&transaction, new_intake)?;
    transaction.commit()?;
    Ok(admitted)
}

pub(super) fn admit_in_transaction(
    connection: &Connection,
    new_intake: NewPromptIntake<'_>,
) -> Result<PromptIntakeAdmission> {
    validate_identity(new_intake.job_id, new_intake.target_thread_id)?;
    storage::ensure_schema(connection)?;
    if let Some(existing) = existing_identity(connection, &new_intake)? {
        crate::ingress::link_prompt_owner(connection, &existing)?;
        return Ok(PromptIntakeAdmission {
            intake: existing,
            created: false,
        });
    }
    let target_thread_id =
        crate::queue::canonical_completed_target(connection, new_intake.target_thread_id)?;
    connection.execute(
        "INSERT INTO codex_prompt_intakes (job_id, target_thread_id, channel_id, \
         owner_user_id, discord_message_id, raw_prompt, auto_queue_when_busy, \
         require_current_mirror, attempt_count, last_error, retry_after, claim_token, \
         claim_expires_at, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, '', 0, NULL, 0, ?, ?)",
        params![
            new_intake.job_id,
            target_thread_id,
            new_intake.channel_id,
            new_intake.owner_user_id,
            new_intake.discord_message_id,
            new_intake.raw_prompt,
            i64::from(new_intake.auto_queue_when_busy),
            i64::from(new_intake.require_current_mirror),
            new_intake.created_at,
            new_intake.created_at,
        ],
    )?;
    let intake = storage::by_job(connection, new_intake.job_id)?
        .ok_or_else(|| StoreError::PromptIntakeNotFound(new_intake.job_id.to_owned()))?;
    crate::ingress::link_prompt_owner(connection, &intake)?;
    Ok(PromptIntakeAdmission {
        intake,
        created: true,
    })
}

pub fn canonicalize_prompt_intake_target(
    path: &Path,
    job_id: &str,
) -> Result<Option<StoredPromptIntake>> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_schema(&transaction)?;
    let Some(intake) = storage::by_job(&transaction, job_id)? else {
        transaction.commit()?;
        return Ok(None);
    };
    let canonical =
        crate::queue::canonical_completed_target(&transaction, &intake.target_thread_id)?;
    if canonical != intake.target_thread_id {
        transaction.execute(
            "UPDATE codex_prompt_intakes SET target_thread_id = ?, updated_at = ? \
             WHERE job_id = ? AND target_thread_id = ?",
            params![canonical, unix_now()?, job_id, intake.target_thread_id],
        )?;
    }
    let current = storage::by_job(&transaction, job_id)?;
    transaction.commit()?;
    Ok(current)
}

pub fn remove_prompt_intake_if_queued(path: &Path, job_id: &str) -> Result<bool> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_schema(&transaction)?;
    let removed = transaction.execute(
        "DELETE FROM codex_prompt_intakes WHERE job_id = ? \
         AND NOT EXISTS (SELECT 1 FROM codex_dead_generation_holds hold \
             WHERE hold.target_thread_id = codex_prompt_intakes.target_thread_id) \
         AND (EXISTS (SELECT 1 FROM codex_turn_queue WHERE job_id = ?) \
              OR EXISTS (SELECT 1 FROM codex_delivery_outbox WHERE job_id = ?))",
        params![job_id, job_id, job_id],
    )? == 1;
    transaction.commit()?;
    Ok(removed)
}

fn existing_identity(
    connection: &Connection,
    new_intake: &NewPromptIntake<'_>,
) -> Result<Option<StoredPromptIntake>> {
    let by_job = storage::by_job(connection, new_intake.job_id)?;
    let by_message = new_intake
        .discord_message_id
        .map(|message_id| storage::by_message(connection, message_id))
        .transpose()?
        .flatten();
    match (by_job, by_message) {
        (Some(by_job), by_message) => {
            let matches = by_message.map_or_else(
                || by_job.discord_message_id == new_intake.discord_message_id,
                |by_message| by_job.job_id == by_message.job_id,
            );
            if matches {
                Ok(Some(by_job))
            } else {
                Err(identity_conflict(new_intake))
            }
        }
        (None, Some(by_message)) => Ok(Some(by_message)),
        (None, None) => Ok(None),
    }
}

fn validate_identity(job_id: &str, target_thread_id: &str) -> Result<()> {
    if job_id.is_empty()
        || job_id.trim() != job_id
        || target_thread_id.is_empty()
        || target_thread_id.trim() != target_thread_id
    {
        return Err(StoreError::InvalidPromptIntakeIdentity {
            job_id: job_id.to_owned(),
            target_thread_id: target_thread_id.to_owned(),
        });
    }
    Ok(())
}

fn identity_conflict(new_intake: &NewPromptIntake<'_>) -> StoreError {
    StoreError::PromptIntakeIdentityConflict {
        job_id: new_intake.job_id.to_owned(),
        discord_message_id: new_intake.discord_message_id,
    }
}

fn unix_now() -> Result<f64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(StoreError::from)?
        .as_secs_f64())
}

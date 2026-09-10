use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{PromptIntakeClaim, StoredPromptIntake, storage};
use crate::queue::{
    ExpectedMirrorMapping, NewQueueJob, QueueEnqueueResult, StoredQueueJob, enqueue_in_transaction,
    ensure_mirror_matches, select_job,
};
use crate::schema::open_initialized;
use crate::{Result, StoreError};

/// Atomically transfers ownership of a claimed prompt from the intake table to
/// the durable turn queue. The queue prompt is intentionally not compared with
/// `raw_prompt`: preprocessing may enrich or otherwise transform it.
pub fn promote_prompt_intake_to_queue(
    path: &Path,
    claim: &PromptIntakeClaim,
    new_job: NewQueueJob<'_>,
    now: f64,
) -> Result<QueueEnqueueResult> {
    if !now.is_finite() {
        return Err(StoreError::InvalidPromptIntakeLease {
            now,
            claim_expires_at: claim.intake.claim_expires_at,
        });
    }
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_schema(&transaction)?;
    let current = storage::by_job(&transaction, &claim.intake.job_id)?
        .ok_or_else(|| claim_lost(&claim.intake.job_id))?;
    crate::dead_generation::ensure_target_available(&transaction, &current.target_thread_id)?;
    crate::dead_generation::ensure_target_available(&transaction, new_job.target_thread_id)?;
    validate_current_claim(&current, claim, now)?;
    validate_queue_identity(&current, &new_job)?;
    validate_existing_occurrence(&transaction, &current, &new_job)?;
    if current.require_current_mirror {
        ensure_mirror_matches(
            &transaction,
            &new_job,
            ExpectedMirrorMapping {
                discord_channel_id: current.channel_id,
                target_thread_id: new_job.target_thread_id,
            },
        )?;
    }
    let enqueued = enqueue_in_transaction(&transaction, new_job)?;
    validate_stored_occurrence(&current, &new_job, &enqueued.job)?;
    crate::ingress::record_new_evidence(&transaction, &new_job)?;
    let removed = transaction.execute(
        "DELETE FROM codex_prompt_intakes WHERE job_id = ? AND claim_token = ? \
         AND claim_expires_at > ?",
        params![current.job_id, claim.claim_token, now],
    )?;
    if removed != 1 {
        return Err(claim_lost(&current.job_id));
    }
    transaction.commit()?;
    Ok(enqueued)
}

fn validate_current_claim(
    current: &StoredPromptIntake,
    claim: &PromptIntakeClaim,
    now: f64,
) -> Result<()> {
    if current.target_thread_id != claim.intake.target_thread_id {
        return Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id: claim.intake.target_thread_id.clone(),
            target_thread_id: current.target_thread_id.clone(),
        });
    }
    let stable_identity_matches = current.job_id == claim.intake.job_id
        && current.channel_id == claim.intake.channel_id
        && current.owner_user_id == claim.intake.owner_user_id
        && current.discord_message_id == claim.intake.discord_message_id
        && current.raw_prompt == claim.intake.raw_prompt
        && current.auto_queue_when_busy == claim.intake.auto_queue_when_busy
        && current.require_current_mirror == claim.intake.require_current_mirror;
    let current_token = current.claim_token.as_deref() == Some(&claim.claim_token)
        && claim.intake.claim_token.as_deref() == Some(&claim.claim_token);
    if !stable_identity_matches || !current_token || current.claim_expires_at <= now {
        return Err(claim_lost(&current.job_id));
    }
    Ok(())
}

fn validate_queue_identity(intake: &StoredPromptIntake, job: &NewQueueJob<'_>) -> Result<()> {
    if !intake.require_current_mirror && job.target_thread_id != intake.target_thread_id {
        return Err(StoreError::ForkHandoffTargetMoved {
            source_thread_id: job.target_thread_id.to_owned(),
            target_thread_id: intake.target_thread_id.clone(),
        });
    }
    if job.job_id != intake.job_id
        || job.channel_id != intake.channel_id
        || job.owner_user_id != intake.owner_user_id
        || job.discord_message_id != intake.discord_message_id
    {
        return Err(identity_conflict(intake));
    }
    Ok(())
}

fn validate_existing_occurrence(
    connection: &Connection,
    intake: &StoredPromptIntake,
    job: &NewQueueJob<'_>,
) -> Result<()> {
    let by_job = connection
        .query_row(
            "SELECT job_id FROM codex_turn_queue WHERE job_id = ?",
            [job.job_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|job_id| select_job(connection, &job_id))
        .transpose()?;
    let by_message = job
        .discord_message_id
        .map(|message_id| {
            connection
                .query_row(
                    "SELECT job_id FROM codex_turn_queue WHERE discord_message_id = ?",
                    [message_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .transpose()?
        .flatten()
        .map(|job_id| select_job(connection, &job_id))
        .transpose()?;
    for existing in [by_job.as_ref(), by_message.as_ref()].into_iter().flatten() {
        validate_stored_occurrence(intake, job, existing)?;
    }
    if let (Some(by_job), Some(by_message)) = (&by_job, &by_message)
        && by_job.job_id != by_message.job_id
    {
        return Err(identity_conflict(intake));
    }
    Ok(())
}

fn validate_stored_occurrence(
    intake: &StoredPromptIntake,
    expected: &NewQueueJob<'_>,
    stored: &StoredQueueJob,
) -> Result<()> {
    if stored.job_id != intake.job_id
        || stored.target_thread_id != expected.target_thread_id
        || stored.channel_id != intake.channel_id
        || stored.owner_user_id != intake.owner_user_id
        || stored.discord_message_id != intake.discord_message_id
        || stored.prompt != expected.prompt
    {
        return Err(identity_conflict(intake));
    }
    Ok(())
}

fn claim_lost(job_id: &str) -> StoreError {
    StoreError::PromptIntakeClaimLost {
        job_id: job_id.to_owned(),
    }
}

fn identity_conflict(intake: &StoredPromptIntake) -> StoreError {
    StoreError::PromptIntakeIdentityConflict {
        job_id: intake.job_id.clone(),
        discord_message_id: intake.discord_message_id,
    }
}

use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{NewPromptIntake, StoredPromptIntake, write::admit_in_transaction};
use crate::claims::BusyChoice;
use crate::schema::open_initialized;
use crate::{Result, StoreError};

pub struct BusyQueueAdmission {
    pub job_id: String,
    /// Only the transaction that accepted the choice may start preparation.
    /// Repeated clicks use the receipt, even after the queue has completed.
    pub intake: Option<StoredPromptIntake>,
}

/// Claim the exact displayed choice, save its prompt, and record acceptance
/// together. No Codex request may run before this transaction commits.
pub fn admit_busy_queue(
    path: &Path,
    choice: &BusyChoice,
    target_thread_id: &str,
    require_current_mirror: bool,
    confirmation_key: &str,
    now: f64,
) -> Result<BusyQueueAdmission> {
    if !now.is_finite() || now < 0.0 || confirmation_key.is_empty() {
        return Err(StoreError::Integrity("invalid busy queue admission".into()));
    }
    let job_id = format!("busy-choice:{}", choice.choice_id);
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if choice.target_thread_id.as_deref() != Some(target_thread_id) {
        return Err(StoreError::BusyChoiceUnavailable(choice.choice_id.clone()));
    }
    let accepted: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM persistent_component_claims \
         WHERE claim_key = ? AND expires_at > ?)",
        params![confirmation_key, now],
        |row| row.get(0),
    )?;
    if accepted {
        crate::ingress::record_busy_owner(&transaction, choice, &job_id, target_thread_id, now)?;
        transaction.commit()?;
        return Ok(BusyQueueAdmission {
            job_id,
            intake: None,
        });
    }
    verify_choice_route(&transaction, choice, require_current_mirror)?;
    let claimed = transaction.execute(
        "UPDATE busy_choices SET claimed_at = ? WHERE choice_id = ? \
         AND claimed_at IS NULL AND expires_at > ? \
         AND owner_user_id = ? AND channel_id = ? AND target_thread_id IS ? \
         AND prompt = ? AND allow_steer = ? AND created_at = ? AND expires_at = ?",
        params![
            now,
            choice.choice_id,
            now,
            choice.owner_user_id,
            choice.channel_id,
            choice.target_thread_id,
            choice.prompt,
            i64::from(choice.allow_steer),
            choice.created_at,
            choice.expires_at,
        ],
    )?;
    if claimed != 1 {
        return Err(StoreError::BusyChoiceUnavailable(choice.choice_id.clone()));
    }
    let admitted = admit_in_transaction(
        &transaction,
        NewPromptIntake {
            job_id: &job_id,
            target_thread_id,
            channel_id: choice.channel_id,
            owner_user_id: Some(choice.owner_user_id),
            discord_message_id: None,
            raw_prompt: &choice.prompt,
            auto_queue_when_busy: true,
            require_current_mirror,
            created_at: now,
        },
    )?;
    if admitted.intake.target_thread_id != target_thread_id {
        return Err(StoreError::Integrity(
            "busy prompt target changed during admission; no request accepted".into(),
        ));
    }
    if !admitted.created {
        return Err(StoreError::Integrity(format!(
            "busy queue {job_id} already has an intake without an acceptance receipt"
        )));
    }
    transaction.execute(
        "INSERT INTO persistent_component_claims (claim_key, created_at, expires_at) \
         VALUES (?, ?, ?) ON CONFLICT(claim_key) DO UPDATE SET \
         created_at = excluded.created_at, expires_at = excluded.expires_at",
        params![confirmation_key, now, choice.expires_at.max(now + 1_800.0)],
    )?;
    crate::ingress::record_busy_owner(&transaction, choice, &job_id, target_thread_id, now)?;
    transaction.commit()?;
    Ok(BusyQueueAdmission {
        job_id,
        intake: Some(admitted.intake),
    })
}

fn verify_choice_route(
    connection: &rusqlite::Connection,
    choice: &BusyChoice,
    require_current_mirror: bool,
) -> Result<()> {
    let mapped: Option<bool> = connection
        .query_row(
            "SELECT require_current_mirror FROM busy_choices WHERE choice_id=?",
            [&choice.choice_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let mapped =
        mapped.ok_or_else(|| StoreError::BusyChoiceUnavailable(choice.choice_id.clone()))?;
    crate::claims::verify_route(
        connection,
        choice.channel_id,
        choice.target_thread_id.as_deref(),
        mapped,
    )?;
    if require_current_mirror != mapped {
        return Err(StoreError::Integrity(
            "busy prompt route mode changed; no request accepted".into(),
        ));
    }
    Ok(())
}

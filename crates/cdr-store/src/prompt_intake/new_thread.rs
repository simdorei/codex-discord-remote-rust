use std::path::Path;

use rusqlite::TransactionBehavior;

use super::{NewPromptIntake, PromptIntakeAdmission, write::admit_in_transaction};
use crate::{Result, StoreError};

/// Persist the returned creation identity and transfer custody of the original
/// first prompt in one transaction, before asynchronous preparation can run.
pub fn admit_prompt_intake_with_ingress(
    path: &Path,
    request: NewPromptIntake<'_>,
    ingress_key: &str,
    generation: i64,
) -> Result<PromptIntakeAdmission> {
    admit_prompt_intake_with_ingress_and_reply(path, request, ingress_key, generation, None)
}

pub fn admit_prompt_intake_with_ingress_and_reply(
    path: &Path,
    request: NewPromptIntake<'_>,
    ingress_key: &str,
    generation: i64,
    reply: Option<crate::new_reply::NewReplySeed<'_>>,
) -> Result<PromptIntakeAdmission> {
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::ingress::verify_new_prompt(&transaction, ingress_key, &request)?;
    let matched: bool = transaction.query_row(
        "SELECT state='executing' AND phase='thread/created' AND target_thread_id=?
         AND json_extract(outcome_json,'$.thread_start_generation')=?
         FROM discord_ingress_journal WHERE ingress_id=?",
        rusqlite::params![request.target_thread_id, generation, ingress_key],
        |row| row.get(0),
    )?;
    if !matched {
        return Err(StoreError::Integrity(
            "thread creation has no matching recorded attempt".into(),
        ));
    }
    crate::queue::mark_managed_target_in_transaction(
        &transaction,
        request.target_thread_id,
        generation,
        request.created_at,
    )?;
    let admitted = admit_in_transaction(&transaction, request)?;
    crate::ingress::link_prompt_owner_by_key(&transaction, ingress_key, &admitted.intake)?;
    if let Some(reply) = reply {
        crate::new_reply::seed_in(
            &transaction,
            ingress_key,
            &admitted.intake.job_id,
            reply.state_db,
            reply.acknowledgement,
        )?;
    }
    transaction.commit()?;
    Ok(admitted)
}

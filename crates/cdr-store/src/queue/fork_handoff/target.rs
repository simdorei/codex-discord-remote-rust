use std::path::Path;

use rusqlite::TransactionBehavior;

use super::{
    AppServerForkHandoff, AppServerForkHandoffError, CompletedAppServerForkHandoff, now, storage,
};
use crate::Result as StoreResult;
use crate::queue::read::select_job;
use crate::schema::open_initialized;

use super::transition::{
    clear_unresolved_notices, move_session_detail, pending_ids, quarantine_observed,
    replace_mapping, retarget_pending, validate_completion,
};

pub fn complete_app_server_fork_handoff(
    path: &Path,
    handoff_id: &str,
    target_thread_id: &str,
    current_generation: i64,
) -> Result<CompletedAppServerForkHandoff, AppServerForkHandoffError> {
    stage_app_server_fork_target(path, handoff_id, target_thread_id)?;
    finalize_app_server_fork_handoff(path, handoff_id, current_generation)
}

pub fn stage_app_server_fork_target(
    path: &Path,
    handoff_id: &str,
    target_thread_id: &str,
) -> Result<AppServerForkHandoff, AppServerForkHandoffError> {
    let target = target_thread_id.trim();
    if handoff_id.is_empty()
        || handoff_id.trim() != handoff_id
        || target.is_empty()
        || target != target_thread_id
    {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let handoff = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff_id.to_owned(),
        }
    })?;
    if let Some(existing_target) = handoff
        .target_thread_id
        .as_deref()
        .or(handoff.observed_target_thread_id.as_deref())
    {
        if existing_target != target {
            return Err(AppServerForkHandoffError::TargetConflict {
                target_thread_id: target.to_owned(),
            });
        }
        transaction.commit()?;
        return Ok(handoff);
    }
    if storage::mark_target_observed(&transaction, handoff_id, target)? != 1 {
        return Err(AppServerForkHandoffError::TargetConflict {
            target_thread_id: target.to_owned(),
        });
    }
    let staged = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        }
    })?;
    transaction.commit()?;
    Ok(staged)
}

pub fn finalize_app_server_fork_handoff(
    path: &Path,
    handoff_id: &str,
    current_generation: i64,
) -> Result<CompletedAppServerForkHandoff, AppServerForkHandoffError> {
    if handoff_id.is_empty() || handoff_id.trim() != handoff_id {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let handoff = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff_id.to_owned(),
        }
    })?;
    if handoff.target_thread_id.is_some() {
        let quarantined_job = handoff
            .ambiguous_job_id
            .as_deref()
            .map(|job_id| select_job(&transaction, job_id))
            .transpose()?;
        transaction.commit()?;
        return Ok(CompletedAppServerForkHandoff {
            handoff,
            quarantined_job,
            retargeted_jobs: Vec::new(),
            applied: false,
        });
    }
    let target = handoff
        .observed_target_thread_id
        .as_deref()
        .ok_or_else(|| AppServerForkHandoffError::ForkTargetNotObserved {
            handoff_id: handoff_id.to_owned(),
        })?;
    crate::dead_generation::ensure_target_available(&transaction, &handoff.source_thread_id)?;
    crate::dead_generation::ensure_target_available(&transaction, target)?;
    if handoff.source_thread_id == target {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    validate_completion(&transaction, &handoff, target)?;
    let now = now()?;
    let pending_ids = pending_ids(&transaction, &handoff.source_thread_id)?;
    clear_unresolved_notices(&transaction, &handoff.source_thread_id)?;
    let quarantined_job = quarantine_observed(&transaction, &handoff, now)?;
    retarget_pending(
        &transaction,
        &handoff.source_thread_id,
        target,
        current_generation,
        now,
    )?;
    crate::prompt_intake::retarget_for_fork(&transaction, &handoff.source_thread_id, target, now)?;
    if (handoff.discord_channel_id, handoff.discord_thread_id) != (0, 0) {
        move_session_detail(&transaction, &handoff.source_thread_id, target)?;
        replace_mapping(&transaction, &handoff, target, now)?;
    }
    storage::mark_completed(&transaction, handoff_id, target, current_generation, now)?;
    let completed = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id.clone(),
        }
    })?;
    let retargeted_jobs = pending_ids
        .iter()
        .map(|job_id| select_job(&transaction, job_id))
        .collect::<StoreResult<Vec<_>>>()?;
    transaction.commit()?;
    Ok(CompletedAppServerForkHandoff {
        handoff: completed,
        quarantined_job,
        retargeted_jobs,
        applied: true,
    })
}

pub fn cancel_app_server_fork_handoff_after_definite_failure(
    path: &Path,
    handoff_id: &str,
) -> Result<bool, AppServerForkHandoffError> {
    if handoff_id.is_empty() || handoff_id.trim() != handoff_id {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let Some(handoff) = storage::by_id(&transaction, handoff_id)? else {
        transaction.commit()?;
        return Ok(false);
    };
    if handoff.fork_failure_ambiguous {
        return Err(AppServerForkHandoffError::AmbiguousForkCannotBeCancelled {
            handoff_id: handoff_id.to_owned(),
        });
    }
    if let Some(target_thread_id) = handoff
        .observed_target_thread_id
        .clone()
        .or(handoff.target_thread_id.clone())
    {
        return Err(AppServerForkHandoffError::ForkTargetAlreadyObserved {
            handoff_id: handoff_id.to_owned(),
            target_thread_id,
        });
    }
    if storage::cancel_unobserved(&transaction, handoff_id)? != 1 {
        return Err(AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        });
    }
    transaction.commit()?;
    Ok(true)
}

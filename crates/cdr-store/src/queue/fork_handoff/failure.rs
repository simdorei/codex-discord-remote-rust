use std::path::Path;

use rusqlite::{TransactionBehavior, params};

pub use super::unresolved_notice::UNRESOLVED_FORK_ERROR_PREFIX;
use super::unresolved_notice::{FailurePhase, stage_unresolved_notices};
use super::{AppServerForkHandoff, AppServerForkHandoffError, bounded_fork_error, storage};
use crate::schema::open_initialized;

pub fn record_app_server_fork_failure(
    path: &Path,
    handoff_id: &str,
    error: &str,
    ambiguous: bool,
) -> Result<AppServerForkHandoff, AppServerForkHandoffError> {
    if handoff_id.is_empty() || handoff_id.trim() != handoff_id {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    let bounded_error = bounded_fork_error(error);
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let handoff = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff_id.to_owned(),
        }
    })?;
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
    if storage::record_fork_failure(&transaction, handoff_id, &bounded_error, ambiguous)? != 1 {
        return Err(AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        });
    }
    if ambiguous {
        stage_unresolved_notices(
            &transaction,
            handoff_id,
            &handoff.source_thread_id,
            &handoff.last_fork_error,
            &bounded_error,
            FailurePhase::ForkOutcome,
        )?;
    }
    let recorded = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        }
    })?;
    transaction.commit()?;
    Ok(recorded)
}

pub fn record_app_server_fork_finalize_failure(
    path: &Path,
    handoff_id: &str,
    error: &str,
) -> Result<AppServerForkHandoff, AppServerForkHandoffError> {
    if handoff_id.is_empty() || handoff_id.trim() != handoff_id {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    let bounded_error = bounded_fork_error(error);
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let handoff = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff_id.to_owned(),
        }
    })?;
    let observed_target = handoff
        .observed_target_thread_id
        .as_deref()
        .ok_or_else(|| AppServerForkHandoffError::ForkTargetNotObserved {
            handoff_id: handoff_id.to_owned(),
        })?;
    if handoff.target_thread_id.is_some() {
        return Err(AppServerForkHandoffError::ForkTargetAlreadyObserved {
            handoff_id: handoff_id.to_owned(),
            target_thread_id: observed_target.to_owned(),
        });
    }
    let updated = transaction.execute(
        "UPDATE codex_thread_fork_handoffs SET last_fork_error = ? \
         WHERE handoff_id = ? AND observed_target_thread_id = ? \
         AND target_thread_id IS NULL",
        params![bounded_error, handoff_id, observed_target],
    )?;
    if updated != 1 {
        return Err(AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        });
    }
    stage_unresolved_notices(
        &transaction,
        handoff_id,
        &handoff.source_thread_id,
        &handoff.last_fork_error,
        &bounded_error,
        FailurePhase::Finalize,
    )?;
    let recorded = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        }
    })?;
    transaction.commit()?;
    Ok(recorded)
}

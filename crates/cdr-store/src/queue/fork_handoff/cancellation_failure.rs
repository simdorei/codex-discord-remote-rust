use std::path::Path;

use rusqlite::{TransactionBehavior, params};

use super::unresolved_notice::{FailurePhase, stage_unresolved_notices};
use super::{AppServerForkHandoff, AppServerForkHandoffError, storage};
use crate::schema::open_initialized;

const CANCELLATION_FAILURE_PREFIX: &str = "[cdr-rust:app-server-fork-cancellation-failure:v1] ";

pub fn record_app_server_fork_cancellation_failure(
    path: &Path,
    handoff_id: &str,
    fork_error: &str,
    cancellation_error: &str,
) -> Result<AppServerForkHandoff, AppServerForkHandoffError> {
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
    if let Some(target_thread_id) = handoff.target_thread_id.as_deref() {
        return Err(AppServerForkHandoffError::ForkTargetAlreadyObserved {
            handoff_id: handoff_id.to_owned(),
            target_thread_id: target_thread_id.to_owned(),
        });
    }
    let combined_error = combined_error(fork_error, cancellation_error, &handoff.last_fork_error);
    let updated = transaction.execute(
        "UPDATE codex_thread_fork_handoffs SET last_fork_error = ?, \
         fork_failure_ambiguous = 1 WHERE handoff_id = ? AND target_thread_id IS NULL",
        params![combined_error, handoff_id],
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
        &combined_error,
        FailurePhase::Cancellation,
    )?;
    let recorded = storage::by_id(&transaction, handoff_id)?.ok_or_else(|| {
        AppServerForkHandoffError::ConflictingIntent {
            source_thread_id: handoff.source_thread_id,
        }
    })?;
    transaction.commit()?;
    Ok(recorded)
}

fn combined_error(fork_error: &str, cancellation_error: &str, previous_error: &str) -> String {
    let base = format!(
        "{CANCELLATION_FAILURE_PREFIX}Fork error: {}\nCancellation error: {}",
        bounded_fragment(fork_error, 320, "fork failed without an error message"),
        bounded_fragment(
            cancellation_error,
            320,
            "handoff cancellation failed without an error message",
        ),
    );
    if previous_error.starts_with(&base) {
        return previous_error.to_owned();
    }
    let previous = bounded_fragment(previous_error, 240, "");
    let combined = if previous.is_empty() {
        base
    } else {
        format!("{base}\nPrevious error: {previous}")
    };
    combined.chars().take(1_000).collect()
}

fn bounded_fragment(value: &str, limit: usize, fallback: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() { fallback } else { value };
    value.chars().take(limit).collect()
}

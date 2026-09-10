use std::path::Path;

use rusqlite::{Transaction, TransactionBehavior, params};

use super::failure::UNRESOLVED_FORK_ERROR_PREFIX;
use super::{AppServerForkHandoff, AppServerForkHandoffError, bounded_fork_error, storage};
use crate::schema::open_initialized;

pub const DEFINITE_FORK_ERROR_PREFIX: &str = "[cdr-rust:app-server-fork-definite:v1] ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedDefiniteAppServerForkFailure {
    pub handoff_id: String,
    pub source_thread_id: String,
    pub last_fork_error: String,
    pub affected_jobs: usize,
}

pub fn record_and_cancel_app_server_fork_handoff_after_definite_failure(
    path: &Path,
    expected: &AppServerForkHandoff,
    error: &str,
) -> Result<RecordedDefiniteAppServerForkFailure, AppServerForkHandoffError> {
    validate_expected(expected)?;
    let bounded_error = bounded_fork_error(error);
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let current = storage::by_id(&transaction, &expected.handoff_id)?
        .ok_or_else(|| conflict(&expected.source_thread_id))?;
    validate_cancellable(&current)?;
    if current != *expected {
        return Err(conflict(&current.source_thread_id));
    }
    let recorded = cancel_in_transaction(&transaction, &current, &bounded_error)?;
    transaction.commit()?;
    Ok(recorded)
}

pub fn repair_legacy_definite_app_server_fork_failures(
    path: &Path,
) -> Result<Vec<RecordedDefiniteAppServerForkFailure>, AppServerForkHandoffError> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    storage::ensure_table(&transaction)?;
    let mut statement = transaction.prepare(
        "SELECT handoff_id FROM codex_thread_fork_handoffs \
         WHERE observed_target_thread_id IS NULL AND target_thread_id IS NULL \
         AND fork_failure_ambiguous = 0 AND last_fork_error != '' \
         ORDER BY created_at, handoff_id",
    )?;
    let handoff_ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    let mut repaired = Vec::with_capacity(handoff_ids.len());
    for handoff_id in handoff_ids {
        let handoff =
            storage::by_id(&transaction, &handoff_id)?.ok_or_else(|| conflict(&handoff_id))?;
        validate_cancellable(&handoff)?;
        let error = bounded_fork_error(&handoff.last_fork_error);
        repaired.push(cancel_in_transaction(&transaction, &handoff, &error)?);
    }
    transaction.commit()?;
    Ok(repaired)
}

fn cancel_in_transaction(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    error: &str,
) -> Result<RecordedDefiniteAppServerForkFailure, AppServerForkHandoffError> {
    if storage::record_fork_failure(transaction, &handoff.handoff_id, error, false)? != 1 {
        return Err(conflict(&handoff.source_thread_id));
    }
    let affected_jobs = stage_definite_notices(
        transaction,
        &handoff.handoff_id,
        &handoff.source_thread_id,
        error,
    )?;
    if storage::cancel_unobserved(transaction, &handoff.handoff_id)? != 1 {
        return Err(conflict(&handoff.source_thread_id));
    }
    Ok(RecordedDefiniteAppServerForkFailure {
        handoff_id: handoff.handoff_id.clone(),
        source_thread_id: handoff.source_thread_id.clone(),
        last_fork_error: error.to_owned(),
        affected_jobs,
    })
}

fn stage_definite_notices(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    source: &str,
    fork_error: &str,
) -> Result<usize, AppServerForkHandoffError> {
    let mut statement = transaction.prepare(
        "SELECT job_id, channel_id, state, last_error FROM codex_turn_queue \
         WHERE target_thread_id = ? AND state IN ('pending', 'starting') \
         ORDER BY created_at, job_id",
    )?;
    let jobs = statement
        .query_map([source], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (job_id, channel_id, state, stored_error) in &jobs {
        let previous_error = previous_non_fork_error(stored_error);
        let queue_error = definite_message(fork_error, &previous_error);
        if transaction.execute(
            "UPDATE codex_turn_queue SET last_error = ?, updated_at = ? \
             WHERE job_id = ? AND target_thread_id = ? AND state = ? AND last_error = ?",
            params![
                queue_error,
                super::now()?,
                job_id,
                source,
                state,
                stored_error,
            ],
        )? != 1
        {
            return Err(conflict(source));
        }
        let notice_id = format!("fork-definite:{handoff_id}:{job_id}");
        let notice_time = super::now()?;
        transaction.execute(
            "INSERT INTO codex_delivery_outbox (delivery_id, job_id, target_thread_id, \
             turn_id, channel_id, content, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(delivery_id) DO NOTHING",
            params![
                notice_id,
                notice_id,
                source,
                format!("fork-definite:{handoff_id}"),
                channel_id,
                definite_notice(fork_error, &previous_error),
                notice_time,
                notice_time,
            ],
        )?;
    }
    Ok(jobs.len())
}

fn validate_expected(expected: &AppServerForkHandoff) -> Result<(), AppServerForkHandoffError> {
    if expected.handoff_id.is_empty()
        || expected.handoff_id.trim() != expected.handoff_id
        || expected.source_thread_id.is_empty()
        || expected.source_thread_id.trim() != expected.source_thread_id
    {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    Ok(())
}

fn validate_cancellable(handoff: &AppServerForkHandoff) -> Result<(), AppServerForkHandoffError> {
    if handoff.fork_failure_ambiguous {
        return Err(AppServerForkHandoffError::AmbiguousForkCannotBeCancelled {
            handoff_id: handoff.handoff_id.clone(),
        });
    }
    if let Some(target_thread_id) = handoff
        .observed_target_thread_id
        .clone()
        .or(handoff.target_thread_id.clone())
    {
        return Err(AppServerForkHandoffError::ForkTargetAlreadyObserved {
            handoff_id: handoff.handoff_id.clone(),
            target_thread_id,
        });
    }
    if handoff.completed_generation.is_some() {
        return Err(conflict(&handoff.source_thread_id));
    }
    Ok(())
}

fn previous_non_fork_error(stored_error: &str) -> String {
    let stored_error = stored_error.trim();
    let generated = [DEFINITE_FORK_ERROR_PREFIX, UNRESOLVED_FORK_ERROR_PREFIX]
        .iter()
        .find_map(|prefix| stored_error.strip_prefix(prefix));
    let previous = generated.map_or(stored_error, |value| {
        value
            .rsplit_once("\nPrevious error: ")
            .map_or("", |(_, previous)| previous)
    });
    if [DEFINITE_FORK_ERROR_PREFIX, UNRESOLVED_FORK_ERROR_PREFIX]
        .iter()
        .any(|prefix| previous.starts_with(prefix))
    {
        String::new()
    } else {
        previous.chars().take(1_000).collect()
    }
}

fn definite_message(fork_error: &str, previous_error: &str) -> String {
    if previous_error.is_empty() {
        format!("{DEFINITE_FORK_ERROR_PREFIX}{fork_error}")
    } else {
        format!("{DEFINITE_FORK_ERROR_PREFIX}{fork_error}\nPrevious error: {previous_error}")
    }
}

fn definite_notice(fork_error: &str, previous_error: &str) -> String {
    let mut content = format!(
        "The Codex ownership fork definitely failed before a target was created. This request remains durable and can be retried safely.\nFork error: {fork_error}"
    );
    if !previous_error.is_empty() {
        content.push_str("\nPrevious error: ");
        content.push_str(previous_error);
    }
    content
}

fn conflict(source_thread_id: &str) -> AppServerForkHandoffError {
    AppServerForkHandoffError::ConflictingIntent {
        source_thread_id: source_thread_id.to_owned(),
    }
}

mod validation;

pub(super) use validation::{
    conflicting_or_existing, mapping_snapshot, validate_begin, validate_completion,
    validate_no_other_inflight, validate_starting_job,
};

use super::{AppServerForkHandoff, AppServerForkHandoffError};
use crate::queue::read::select_job;
use crate::queue::{QUARANTINED_ERROR_PREFIX, QUARANTINED_TURN_PREFIX, StoredQueueJob};
use rusqlite::{Transaction, params};

pub(super) fn pending_ids(
    transaction: &Transaction<'_>,
    source: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT job_id FROM codex_turn_queue WHERE target_thread_id = ? \
         AND state = 'pending' ORDER BY created_at, job_id",
    )?;
    statement
        .query_map([source], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()
}

pub(super) fn clear_unresolved_notices(
    transaction: &Transaction<'_>,
    source: &str,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "DELETE FROM codex_delivery_outbox WHERE delivery_id IN (\
             SELECT CASE state WHEN 'starting' \
                 THEN 'fork-unresolved-starting:' || job_id \
                 ELSE 'fork-unresolved:' || job_id END \
             FROM codex_turn_queue WHERE target_thread_id = ? \
                 AND state IN ('pending', 'starting')\
         )",
        [source],
    )?;
    transaction.execute(
        "DELETE FROM codex_delivery_outbox WHERE delivery_id IN (\
             SELECT 'fork-unresolved-intake:' || job_id \
             FROM codex_prompt_intakes WHERE target_thread_id = ?\
         )",
        [source],
    )?;
    Ok(())
}

pub(super) fn quarantine_observed(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    now: f64,
) -> Result<Option<StoredQueueJob>, AppServerForkHandoffError> {
    let Some(job_id) = handoff.ambiguous_job_id.as_deref() else {
        return Ok(None);
    };
    let before = select_job(transaction, job_id)?;
    let turn_id = format!("{QUARANTINED_TURN_PREFIX}{}", handoff.handoff_id);
    let previous_error: String = before.last_error.trim().chars().take(1_000).collect();
    let error = quarantine_message(&handoff.quarantine_reason, &previous_error);
    let updated = transaction.execute(
        "UPDATE codex_turn_queue SET state = 'running', turn_id = ?, last_error = ?, \
         updated_at = ? WHERE job_id = ? AND target_thread_id = ? \
         AND app_server_generation = ? AND state = 'starting' AND turn_id IS NULL",
        params![
            turn_id,
            error,
            now,
            job_id,
            handoff.source_thread_id,
            handoff.expected_generation,
        ],
    )?;
    if updated != 1 {
        return Err(AppServerForkHandoffError::StaleStartingJob {
            job_id: job_id.to_owned(),
        });
    }
    transaction.execute(
        "INSERT INTO codex_delivery_outbox (delivery_id, job_id, \
         target_thread_id, turn_id, channel_id, content, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(delivery_id) DO NOTHING",
        params![
            format!("quarantine:{job_id}"),
            job_id,
            handoff.source_thread_id,
            turn_id,
            before.channel_id,
            terminal_quarantine_content(&handoff.quarantine_reason, &previous_error),
            now,
            now,
        ],
    )?;
    Ok(Some(select_job(transaction, job_id)?))
}

pub(super) fn move_session_detail(
    transaction: &Transaction<'_>,
    source: &str,
    target: &str,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE session_mirror_details SET codex_thread_id = ? \
         WHERE codex_thread_id = ?",
        params![target, source],
    )?;
    Ok(())
}

pub(super) fn retarget_pending(
    transaction: &Transaction<'_>,
    source: &str,
    target: &str,
    generation: i64,
    now: f64,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "UPDATE codex_turn_queue SET target_thread_id = ?, app_server_generation = ?, \
         last_error = '', updated_at = ? WHERE target_thread_id = ? AND state = 'pending'",
        params![target, generation, now, source],
    )?;
    Ok(())
}

pub(super) fn replace_mapping(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    target: &str,
    now: f64,
) -> Result<(), AppServerForkHandoffError> {
    let updated = transaction.execute(
        "UPDATE mirror_threads SET codex_thread_id = ?, updated_at = ? \
         WHERE codex_thread_id = ? AND discord_channel_id = ? AND discord_thread_id = ?",
        params![
            target,
            now,
            handoff.source_thread_id,
            handoff.discord_channel_id,
            handoff.discord_thread_id,
        ],
    )?;
    if updated != 1 {
        return Err(stale_mapping(&handoff.source_thread_id));
    }
    Ok(())
}

fn stale_mapping(source: &str) -> AppServerForkHandoffError {
    AppServerForkHandoffError::MissingOrStaleMapping {
        source_thread_id: source.to_owned(),
    }
}

fn quarantine_message(reason: &str, previous_error: &str) -> String {
    if previous_error.is_empty() {
        format!("{QUARANTINED_ERROR_PREFIX}{reason}")
    } else {
        format!("{QUARANTINED_ERROR_PREFIX}{reason}\nPrevious error: {previous_error}")
    }
}

fn terminal_quarantine_content(reason: &str, previous_error: &str) -> String {
    let mut content = format!(
        "Codex request start result is uncertain, so it was not retried to prevent a duplicate response.\nQuarantine reason: {reason}"
    );
    if !previous_error.is_empty() {
        content.push_str("\nPrevious error: ");
        content.push_str(previous_error);
    }
    content
}

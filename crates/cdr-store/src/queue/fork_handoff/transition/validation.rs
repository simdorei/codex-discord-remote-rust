use super::super::{
    AppServerForkHandoff, AppServerForkHandoffError, NewAppServerForkHandoff,
    STARTING_ATTEMPT_LEASE_SECONDS, bounded_reason, storage,
};
use rusqlite::{OptionalExtension, Transaction, params};

pub(in crate::queue::fork_handoff) fn validate_begin(
    request: &NewAppServerForkHandoff<'_>,
) -> Result<(), AppServerForkHandoffError> {
    let identities = [
        request.handoff_id,
        request.source_thread_id,
        request.ambiguous_job_id.unwrap_or("valid-placeholder"),
    ];
    if identities
        .iter()
        .any(|value| value.is_empty() || value.trim() != *value)
    {
        return Err(AppServerForkHandoffError::InvalidIdentity);
    }
    Ok(())
}

pub(in crate::queue::fork_handoff) fn conflicting_or_existing(
    transaction: &Transaction<'_>,
    request: &NewAppServerForkHandoff<'_>,
) -> Result<Option<AppServerForkHandoff>, AppServerForkHandoffError> {
    if let Some(existing) = storage::by_id(transaction, request.handoff_id)? {
        if existing.ambiguous_job_id.as_deref() == request.ambiguous_job_id
            && existing.source_thread_id == request.source_thread_id
            && existing.expected_generation == request.expected_generation
            && existing.quarantine_reason == bounded_reason(request.quarantine_reason)
        {
            return Ok(Some(existing));
        }
        return Err(conflict(request.source_thread_id));
    }
    if storage::by_source(transaction, request.source_thread_id)?.is_some()
        || match request.ambiguous_job_id {
            Some(job_id) => storage::by_ambiguous_job(transaction, job_id)?.is_some(),
            None => false,
        }
    {
        return Err(conflict(request.source_thread_id));
    }
    Ok(None)
}

pub(in crate::queue::fork_handoff) fn validate_starting_job(
    transaction: &Transaction<'_>,
    request: &NewAppServerForkHandoff<'_>,
    lease_now: Option<f64>,
) -> Result<(), AppServerForkHandoffError> {
    let Some(job_id) = request.ambiguous_job_id else {
        return Ok(());
    };
    let observation = transaction
        .query_row(
            "SELECT updated_at, last_error FROM codex_turn_queue WHERE job_id = ? \
         AND target_thread_id = ? AND app_server_generation = ? \
         AND state = 'starting' AND turn_id IS NULL",
            params![
                job_id,
                request.source_thread_id,
                request.expected_generation
            ],
            |row| Ok((row.get::<_, f64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((updated_at, last_error)) = observation else {
        return Err(AppServerForkHandoffError::StaleStartingJob {
            job_id: job_id.to_owned(),
        });
    };
    if lease_now.is_some_and(|now| {
        last_error.trim().is_empty() && updated_at > now - STARTING_ATTEMPT_LEASE_SECONDS
    }) {
        return Err(AppServerForkHandoffError::StartingAttemptLeaseActive {
            job_id: job_id.to_owned(),
        });
    }
    Ok(())
}

pub(in crate::queue::fork_handoff) fn validate_no_other_inflight(
    transaction: &Transaction<'_>,
    source: &str,
    excluded_job_id: Option<&str>,
) -> Result<(), AppServerForkHandoffError> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM codex_turn_queue WHERE target_thread_id = ? \
         AND state IN ('starting', 'running') AND (? IS NULL OR job_id != ?)",
        params![source, excluded_job_id, excluded_job_id],
        |row| row.get(0),
    )?;
    if count != 0 {
        return Err(AppServerForkHandoffError::AdditionalInFlight {
            source_thread_id: source.to_owned(),
        });
    }
    Ok(())
}

pub(in crate::queue::fork_handoff) fn mapping_snapshot(
    transaction: &Transaction<'_>,
    source: &str,
) -> Result<(i64, i64), AppServerForkHandoffError> {
    let mapping = transaction
        .query_row(
            "SELECT discord_channel_id, discord_thread_id FROM mirror_threads \
             WHERE codex_thread_id = ?",
            [source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((channel_id, thread_id)) = mapping else {
        return Ok((0, 0));
    };
    if (channel_id, thread_id) == (0, 0) {
        return Err(stale_mapping(source));
    }
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM mirror_threads WHERE discord_thread_id = ?",
        [thread_id],
        |row| row.get(0),
    )?;
    if count != 1 {
        return Err(stale_mapping(source));
    }
    Ok((channel_id, thread_id))
}

pub(in crate::queue::fork_handoff) fn validate_completion(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    target: &str,
) -> Result<(), AppServerForkHandoffError> {
    let current = mapping_snapshot(transaction, &handoff.source_thread_id)?;
    if current != (handoff.discord_channel_id, handoff.discord_thread_id) {
        return Err(stale_mapping(&handoff.source_thread_id));
    }
    validate_stage_target(transaction, handoff, target)?;
    validate_no_other_inflight(
        transaction,
        &handoff.source_thread_id,
        handoff.ambiguous_job_id.as_deref(),
    )?;
    if let Some(job_id) = handoff.ambiguous_job_id.as_deref() {
        let request = NewAppServerForkHandoff {
            handoff_id: &handoff.handoff_id,
            ambiguous_job_id: Some(job_id),
            source_thread_id: &handoff.source_thread_id,
            expected_generation: handoff.expected_generation,
            quarantine_reason: &handoff.quarantine_reason,
        };
        validate_starting_job(transaction, &request, None)?;
    }
    Ok(())
}

pub(super) fn validate_stage_target(
    transaction: &Transaction<'_>,
    handoff: &AppServerForkHandoff,
    target: &str,
) -> Result<(), AppServerForkHandoffError> {
    let target_used: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM mirror_threads WHERE codex_thread_id = ?) \
         OR EXISTS(SELECT 1 FROM codex_turn_queue WHERE target_thread_id = ?) \
         OR EXISTS(SELECT 1 FROM session_mirror_details WHERE codex_thread_id = ?) \
         OR EXISTS(SELECT 1 FROM codex_prompt_intakes WHERE target_thread_id = ?) \
         OR EXISTS(SELECT 1 FROM codex_thread_fork_handoffs \
             WHERE handoff_id != ? AND (observed_target_thread_id = ? OR target_thread_id = ?))",
        params![
            target,
            target,
            target,
            target,
            handoff.handoff_id,
            target,
            target
        ],
        |row| row.get(0),
    )?;
    if target_used || storage::managed_target(transaction, target)? {
        return Err(AppServerForkHandoffError::TargetConflict {
            target_thread_id: target.to_owned(),
        });
    }
    Ok(())
}

fn conflict(source: &str) -> AppServerForkHandoffError {
    AppServerForkHandoffError::ConflictingIntent {
        source_thread_id: source.to_owned(),
    }
}

fn stale_mapping(source: &str) -> AppServerForkHandoffError {
    AppServerForkHandoffError::MissingOrStaleMapping {
        source_thread_id: source.to_owned(),
    }
}

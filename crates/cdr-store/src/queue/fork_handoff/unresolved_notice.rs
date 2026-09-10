mod format;

use rusqlite::{Transaction, params};

use super::AppServerForkHandoffError;
use format::{prior_error, unresolved_message, unresolved_notice};

pub const UNRESOLVED_FORK_ERROR_PREFIX: &str = "[cdr-rust:app-server-fork-unresolved:v1] ";

#[derive(Clone, Copy)]
pub(super) enum FailurePhase {
    ForkOutcome,
    Finalize,
    Cancellation,
}

pub(super) fn stage_unresolved_notices(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    source: &str,
    previous_fork_error: &str,
    fork_error: &str,
    phase: FailurePhase,
) -> Result<(), AppServerForkHandoffError> {
    stage_queue_notices(
        transaction,
        handoff_id,
        source,
        previous_fork_error,
        fork_error,
        phase,
    )?;
    stage_intake_notices(
        transaction,
        handoff_id,
        source,
        previous_fork_error,
        fork_error,
        phase,
    )
}

fn stage_queue_notices(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    source: &str,
    previous_fork_error: &str,
    fork_error: &str,
    phase: FailurePhase,
) -> Result<(), AppServerForkHandoffError> {
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
    for (job_id, channel_id, state, stored_error) in jobs {
        let notice_id = if state == "starting" {
            format!("fork-unresolved-starting:{job_id}")
        } else {
            format!("fork-unresolved:{job_id}")
        };
        let notice_job_id = if state == "starting" {
            notice_id.clone()
        } else {
            job_id.clone()
        };
        update_queue_marker(
            transaction,
            source,
            &job_id,
            &state,
            &stored_error,
            previous_fork_error,
            fork_error,
        )?;
        stage_notice(
            transaction,
            &Notice {
                delivery_id: &notice_id,
                job_id: &notice_job_id,
                target_thread_id: source,
                turn_id: &format!("fork-unresolved:{handoff_id}"),
                channel_id,
                content: &unresolved_notice(
                    fork_error,
                    &prior_error(&stored_error, previous_fork_error),
                    phase,
                ),
                previously_staged: stored_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX),
            },
        )?;
    }
    Ok(())
}

fn stage_intake_notices(
    transaction: &Transaction<'_>,
    handoff_id: &str,
    source: &str,
    previous_fork_error: &str,
    fork_error: &str,
    phase: FailurePhase,
) -> Result<(), AppServerForkHandoffError> {
    let mut statement = transaction.prepare(
        "SELECT job_id, channel_id, last_error FROM codex_prompt_intakes \
         WHERE target_thread_id = ? ORDER BY created_at, job_id",
    )?;
    let intakes = statement
        .query_map([source], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (job_id, channel_id, stored_error) in intakes {
        let previously_staged = stored_error.starts_with(UNRESOLVED_FORK_ERROR_PREFIX);
        let previous_error = prior_error(&stored_error, previous_fork_error);
        let marker = unresolved_message(fork_error, &previous_error);
        if transaction.execute(
            "UPDATE codex_prompt_intakes SET last_error = ?, updated_at = ? \
             WHERE job_id = ? AND target_thread_id = ? AND last_error = ?",
            params![marker, super::now()?, job_id, source, stored_error],
        )? != 1
        {
            return Err(conflict(source));
        }
        let notice_id = format!("fork-unresolved-intake:{job_id}");
        stage_notice(
            transaction,
            &Notice {
                delivery_id: &notice_id,
                job_id: &notice_id,
                target_thread_id: source,
                turn_id: &format!("fork-unresolved-intake:{handoff_id}"),
                channel_id,
                content: &unresolved_notice(fork_error, &previous_error, phase),
                previously_staged,
            },
        )?;
    }
    Ok(())
}

fn update_queue_marker(
    transaction: &Transaction<'_>,
    source: &str,
    job_id: &str,
    state: &str,
    stored_error: &str,
    previous_fork_error: &str,
    fork_error: &str,
) -> Result<(), AppServerForkHandoffError> {
    let marker = unresolved_message(fork_error, &prior_error(stored_error, previous_fork_error));
    if transaction.execute(
        "UPDATE codex_turn_queue SET last_error = ?, updated_at = ? \
         WHERE job_id = ? AND target_thread_id = ? AND state = ? AND last_error = ?",
        params![marker, super::now()?, job_id, source, state, stored_error],
    )? != 1
    {
        return Err(conflict(source));
    }
    Ok(())
}

struct Notice<'a> {
    delivery_id: &'a str,
    job_id: &'a str,
    target_thread_id: &'a str,
    turn_id: &'a str,
    channel_id: i64,
    content: &'a str,
    previously_staged: bool,
}

fn stage_notice(
    transaction: &Transaction<'_>,
    notice: &Notice<'_>,
) -> Result<(), AppServerForkHandoffError> {
    let now = super::now()?;
    if notice.previously_staged {
        transaction.execute(
            "UPDATE codex_delivery_outbox SET content = ?, updated_at = ? \
             WHERE delivery_id = ? AND content != ?",
            params![notice.content, now, notice.delivery_id, notice.content],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO codex_delivery_outbox (delivery_id, job_id, target_thread_id, \
             turn_id, channel_id, content, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(delivery_id) DO NOTHING",
            params![
                notice.delivery_id,
                notice.job_id,
                notice.target_thread_id,
                notice.turn_id,
                notice.channel_id,
                notice.content,
                now,
                now,
            ],
        )?;
    }
    Ok(())
}

fn conflict(source: &str) -> AppServerForkHandoffError {
    AppServerForkHandoffError::ConflictingIntent {
        source_thread_id: source.to_owned(),
    }
}

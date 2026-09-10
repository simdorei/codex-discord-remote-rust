use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::super::read::select_job;
use super::super::{ExpectedMirrorMapping, NewQueueJob, QueueEnqueueResult, StoredQueueJob};
use crate::schema::open_initialized;
use crate::{Result, StoreError};

pub fn enqueue(path: &Path, new_job: NewQueueJob<'_>) -> Result<QueueEnqueueResult> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = enqueue_in_transaction(&transaction, new_job)?;
    transaction.commit()?;
    Ok(result)
}

pub fn enqueue_if_mirror_matches(
    path: &Path,
    new_job: NewQueueJob<'_>,
    expected: ExpectedMirrorMapping<'_>,
) -> Result<QueueEnqueueResult> {
    let mut connection = open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    ensure_mirror_matches(&transaction, &new_job, expected)?;
    let result = enqueue_in_transaction(&transaction, new_job)?;
    transaction.commit()?;
    Ok(result)
}

pub(crate) fn enqueue_in_transaction(
    transaction: &Connection,
    new_job: NewQueueJob<'_>,
) -> Result<QueueEnqueueResult> {
    super::super::fork_handoff::ensure_no_unresolved_handoff(
        transaction,
        new_job.target_thread_id,
    )?;
    super::super::fork_handoff::ensure_source_not_moved(transaction, new_job.target_thread_id)?;
    let created = transaction.execute(
        "INSERT OR IGNORE INTO codex_turn_queue (job_id, target_thread_id, channel_id, \
         owner_user_id, discord_message_id, app_server_generation, prompt, queued, ack_sent, \
         state, attempt_count, baseline_turn_ids, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, '[]', ?, ?)",
        params![
            new_job.job_id,
            new_job.target_thread_id,
            new_job.channel_id,
            new_job.owner_user_id,
            new_job.discord_message_id,
            new_job.app_server_generation,
            new_job.prompt,
            i64::from(new_job.queued),
            i64::from(new_job.ack_sent),
            new_job.created_at,
            new_job.created_at,
        ],
    )? == 1;
    let job = if created || new_job.discord_message_id.is_none() {
        select_job(transaction, new_job.job_id)?
    } else {
        let message_id = new_job
            .discord_message_id
            .ok_or_else(|| StoreError::QueueJobNotFound(new_job.job_id.into()))?;
        select_by_message(transaction, message_id)?
    };
    Ok(QueueEnqueueResult { job, created })
}

pub(crate) fn ensure_mirror_matches(
    connection: &Connection,
    new_job: &NewQueueJob<'_>,
    expected: ExpectedMirrorMapping<'_>,
) -> Result<()> {
    let actual = strict_mirror_target(connection, expected.discord_channel_id)?;
    if actual.as_deref() != Some(expected.target_thread_id)
        || new_job.channel_id != expected.discord_channel_id
        || new_job.target_thread_id != expected.target_thread_id
    {
        return Err(StoreError::MirrorMappingChanged {
            discord_channel_id: expected.discord_channel_id,
            expected_target_thread_id: expected.target_thread_id.to_owned(),
            actual_target_thread_id: actual,
        });
    }
    Ok(())
}

fn strict_mirror_target(connection: &Connection, channel_id: i64) -> Result<Option<String>> {
    let exact = mirror_candidates(
        connection,
        "SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id = ? LIMIT 2",
        channel_id,
    )?;
    if exact.len() == 1 {
        return Ok(exact.into_iter().next());
    }
    if !exact.is_empty() {
        return Ok(None);
    }
    let project = mirror_candidates(
        connection,
        "SELECT codex_thread_id FROM mirror_threads WHERE discord_channel_id = ? \
         ORDER BY updated_at DESC LIMIT 2",
        channel_id,
    )?;
    Ok((project.len() == 1).then(|| project[0].clone()))
}

fn mirror_candidates(connection: &Connection, sql: &str, channel_id: i64) -> Result<Vec<String>> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map([channel_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn select_by_message(connection: &Connection, message_id: i64) -> Result<StoredQueueJob> {
    let job_id = connection
        .query_row(
            "SELECT job_id FROM codex_turn_queue WHERE discord_message_id = ?",
            [message_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    select_job(
        connection,
        job_id
            .as_deref()
            .ok_or_else(|| StoreError::QueueJobNotFound(message_id.to_string()))?,
    )
}

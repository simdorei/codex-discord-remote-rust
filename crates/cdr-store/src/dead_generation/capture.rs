use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::queue::{QueueJobState, StoredQueueJob};
use crate::{Result, StoreError};

#[derive(Clone, Copy)]
pub struct DeadGenerationCapture<'a> {
    pub runtime_id: &'a str,
    pub generation: i64,
    pub snapshot_json: &'a str,
    pub affected_targets: &'a [String],
    pub startup_channel_id: Option<i64>,
    pub has_unscoped_requests: bool,
    pub now: f64,
}

/// Commit the exact incident, holds, generation seal, and notices together.
/// The receipt survives outbox delivery, so capture retries cannot restage it.
pub fn capture_dead_generation(path: &Path, capture: DeadGenerationCapture<'_>) -> Result<bool> {
    validate(&capture)?;
    let mut connection = crate::schema::open_initialized(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let active: Option<String> = transaction
        .query_row(
            "SELECT runtime_id FROM codex_app_server_runtime WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if active.as_deref() != Some(capture.runtime_id) {
        return Err(StoreError::Integrity(
            "app-server incident runtime identity is stale".into(),
        ));
    }
    let previous: Option<String> = transaction.query_row(
        "SELECT snapshot_json FROM codex_dead_generation_incidents WHERE runtime_id = ? AND generation = ?",
        params![capture.runtime_id, capture.generation], |row| row.get(0),
    ).optional()?;
    if let Some(previous) = previous {
        if previous != capture.snapshot_json {
            return Err(StoreError::Integrity(
                "dead-generation receipt snapshot changed".into(),
            ));
        }
        transaction.commit()?;
        return Ok(false);
    }
    let jobs = crate::queue::read::all_jobs(&transaction)?
        .into_iter()
        .filter(|job| {
            job.app_server_generation == capture.generation
                && matches!(job.state, QueueJobState::Starting | QueueJobState::Running)
        })
        .collect::<Vec<_>>();
    let targets = capture
        .affected_targets
        .iter()
        .cloned()
        .chain(jobs.iter().map(|job| job.target_thread_id.clone()))
        .collect::<BTreeSet<_>>();
    transaction.execute(
        "INSERT INTO codex_dead_generation_incidents \
         (runtime_id, generation, snapshot_json, queue_jobs_json, created_at) VALUES (?, ?, ?, ?, ?)",
        params![capture.runtime_id, capture.generation, capture.snapshot_json,
            serde_json::to_string(&jobs)?, capture.now],
    )?;
    for (index, target) in targets.iter().enumerate() {
        transaction.execute(
            "INSERT INTO codex_dead_generation_holds \
             (target_thread_id, runtime_id, generation, created_at) VALUES (?, ?, ?, ?) \
             ON CONFLICT(target_thread_id) DO NOTHING",
            params![target, capture.runtime_id, capture.generation, capture.now],
        )?;
        stage_notice(&transaction, &capture, target, index, &jobs)?;
    }
    if capture.has_unscoped_requests {
        stage_notice(&transaction, &capture, "", targets.len(), &jobs)?;
    }
    transaction.commit()?;
    Ok(true)
}

fn validate(capture: &DeadGenerationCapture<'_>) -> Result<()> {
    if capture.runtime_id.is_empty()
        || capture.generation <= 0
        || !capture.now.is_finite()
        || capture.now < 0.0
        || capture
            .affected_targets
            .iter()
            .any(|target| target.trim().is_empty())
    {
        return Err(StoreError::Integrity(
            "invalid dead-generation capture identity".into(),
        ));
    }
    let _: serde_json::Value = serde_json::from_str(capture.snapshot_json)?;
    Ok(())
}

fn stage_notice(
    connection: &Connection,
    capture: &DeadGenerationCapture<'_>,
    target: &str,
    index: usize,
    jobs: &[StoredQueueJob],
) -> Result<()> {
    let queued = jobs
        .iter()
        .find(|job| job.target_thread_id == target)
        .map(|job| job.channel_id);
    let mapped: Option<i64> = connection.query_row(
        "SELECT CASE WHEN discord_thread_id = 0 THEN discord_channel_id ELSE discord_thread_id END \
         FROM mirror_threads WHERE codex_thread_id = ?",
        [target], |row| row.get(0),
    ).optional()?;
    let channel = queued
        .or(mapped)
        .or(capture.startup_channel_id)
        .filter(|channel| *channel > 0)
        .ok_or_else(|| {
            StoreError::Integrity("dead-generation notice has no usable channel".into())
        })?;
    let identity = format!(
        "dead-generation:{}:{}:{index}",
        capture.runtime_id, capture.generation
    );
    let content = if target.is_empty() {
        "The Codex app-server stopped with an unresolved request that could not be assigned to a conversation. Its full details were saved locally for manual review; the request was not automatically replayed."
    } else {
        "The Codex app-server stopped while a request result was uncertain. This conversation is on hold; its saved requests were not retried. Manual review is required before continuing here. Other conversations can continue."
    };
    connection.execute(
        "INSERT INTO codex_delivery_outbox (delivery_id, job_id, target_thread_id, turn_id, \
         channel_id, content, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            identity,
            identity,
            target,
            identity,
            channel,
            content,
            capture.now,
            capture.now
        ],
    )?;
    Ok(())
}

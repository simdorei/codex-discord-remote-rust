use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cdr_store::queue::{QueueJobState, list};
use rusqlite::{Connection, params};

use crate::queue_runner::{BackendFailure, QueueCoordinator, TurnBackend, pending_retry_due_at};

use super::SoakResult;
use super::fake_backend::FakeTurnBackend;
use super::report::SoakCounters;

const BLOCKED_TARGET: &str = "thread-a";
const HEALTHY_TARGET: &str = "thread-b";
const OUTAGE: &str = "deterministic resume outage for thread-a";

pub(super) async fn run_recovery_probe(
    db: &Path,
    queue: &QueueCoordinator<FakeTurnBackend>,
    backend: &Arc<FakeTurnBackend>,
    counters: &mut SoakCounters,
) -> SoakResult<()> {
    counters.recovery_injections += 1;
    backend.set_resume_unavailable(BLOCKED_TARGET, true);
    let blocked = queue
        .submit(
            BLOCKED_TARGET,
            200,
            300,
            Some(9_000_001),
            "offline recovery probe",
        )
        .await?;
    counters.queue_submitted += 1;
    if blocked.queued
        && blocked.turn_id.is_none()
        && blocked.warning == Some(BackendFailure::definite(OUTAGE))
    {
        counters.queue_accepted_with_warning += 1;
    } else {
        return Err(contract_error(
            "outage was not durably accepted with its exact warning",
        ));
    }

    let healthy = queue
        .submit(
            HEALTHY_TARGET,
            201,
            300,
            Some(9_000_002),
            "offline recovery probe",
        )
        .await?;
    counters.queue_submitted += 1;
    if healthy.warning.is_none()
        && healthy.turn_id.is_some()
        && backend.is_active(HEALTHY_TARGET).await
        && !backend.is_active(BLOCKED_TARGET).await
    {
        counters.healthy_target_progressed += 1;
    } else {
        return Err(contract_error(
            "healthy target did not progress while its peer was in durable backoff",
        ));
    }

    let (reused_generation, restart_epoch) = backend.restart_reusing_generation();
    if reused_generation == backend.generation() {
        counters.generation_reuse_restarts = restart_epoch;
    }
    backend.set_resume_unavailable(BLOCKED_TARGET, false);

    let resumes_before = backend.resume_attempts(BLOCKED_TARGET).await;
    let deferred = queue.recover().await?;
    counters.recovery_attempts += 1;
    if deferred.started == 0
        && backend.resume_attempts(BLOCKED_TARGET).await == resumes_before
        && !backend.is_active(BLOCKED_TARGET).await
        && backend.is_active(HEALTHY_TARGET).await
    {
        counters.recovery_deferred_until_due += 1;
    } else {
        return Err(contract_error(
            "pending outage job was retried before its durable deadline",
        ));
    }

    advance_retry_clock_to_due(db, BLOCKED_TARGET)?;
    counters.retry_clock_advances += 1;
    let recovered = queue.recover().await?;
    counters.recovery_attempts += 1;
    if recovered.unavailable_targets.is_empty()
        && recovered.started == 1
        && backend.resume_attempts(BLOCKED_TARGET).await == resumes_before + 1
        && backend.is_active(BLOCKED_TARGET).await
        && backend.is_active(HEALTHY_TARGET).await
    {
        counters.recovery_successes += 1;
    } else {
        return Err(contract_error(
            "outage job did not recover exactly once after its durable deadline",
        ));
    }

    let starts_after_recovery = backend.starts(BLOCKED_TARGET).await;
    let idempotency_check = queue.recover().await?;
    counters.recovery_attempts += 1;
    if idempotency_check.started != 0
        || !idempotency_check.unavailable_targets.is_empty()
        || backend.starts(BLOCKED_TARGET).await != starts_after_recovery
    {
        return Err(contract_error(
            "a later recovery tick started the recovered queue job again",
        ));
    }
    complete_recovered_jobs(queue, backend, counters).await
}

fn advance_retry_clock_to_due(db: &Path, target: &str) -> SoakResult<()> {
    let job = list(db)?
        .into_iter()
        .find(|job| job.target_thread_id == target && job.state == QueueJobState::Pending)
        .ok_or_else(|| contract_error("pending retry job was not preserved"))?;
    let due_at = pending_retry_due_at(job.attempt_count, &job.last_error, job.updated_at)
        .ok_or_else(|| contract_error("pending retry job had no production retry deadline"))?;
    let retry_delay = due_at - job.updated_at;
    let wall_now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs_f64();
    let shifted_updated_at = wall_now - retry_delay;
    let changed = Connection::open(db)?.execute(
        "UPDATE codex_turn_queue SET updated_at = ?1 WHERE job_id = ?2 AND state = 'pending'",
        params![shifted_updated_at, job.job_id],
    )?;
    if changed != 1 {
        return Err(contract_error(
            "offline retry clock could not advance the preserved pending job",
        ));
    }
    Ok(())
}

async fn complete_recovered_jobs(
    queue: &QueueCoordinator<FakeTurnBackend>,
    backend: &FakeTurnBackend,
    counters: &mut SoakCounters,
) -> SoakResult<()> {
    for target in [BLOCKED_TARGET, HEALTHY_TARGET] {
        let turn_id = backend.finish_active(target).await?;
        let delivery = queue
            .stage_turn_completion(target, &turn_id, &format!("recovery:{target}"))
            .await?;
        if delivery.is_some() {
            counters.queue_completed += 1;
            counters.outbox_staged += 1;
        } else {
            return Err(contract_error("recovered queue job was not durably staged"));
        }
    }
    Ok(())
}

fn contract_error(message: &'static str) -> Box<dyn std::error::Error + Send + Sync> {
    io::Error::other(message).into()
}

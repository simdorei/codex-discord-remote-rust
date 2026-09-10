use std::time::{Duration, Instant};

use super::SoakResult;
use super::config::SoakConfig;
use super::harness::OfflineHarness;
use super::report::{
    MODE, ProgressWriter, SUMMARY_SCHEMA, SoakAssertions, SoakCounters, SoakSummary, write_summary,
};
use super::schedule::SCHEDULE_VERSION;
use super::success_tracker::TRACKER_BACKEND;

const CYCLE_INTERVAL: Duration = Duration::from_secs(1);

pub async fn run(config: &SoakConfig) -> SoakResult<SoakSummary> {
    let state = tempfile::tempdir()?;
    let mut harness = OfflineHarness::create(state.path(), config.seed)?;
    let mut progress = ProgressWriter::create(&config.events, config.duration(), config.seed)?;
    let mut counters = SoakCounters::default();
    let started = Instant::now();
    let mut cycles = 0_u64;
    progress.record(Duration::ZERO, cycles, &counters, "running")?;
    let workload = execute_workload(
        config,
        &mut harness,
        &mut progress,
        &mut counters,
        &mut cycles,
        started,
    )
    .await;
    let elapsed = started.elapsed();
    let schedule_digest = harness.schedule_digest();
    let assertions = build_assertions(cycles, &counters, &schedule_digest);
    let failure_reason = match workload {
        Ok(()) if assertions.all_passed() => None,
        Ok(()) => Some("assertion_failed"),
        Err(error) => {
            eprintln!("offline_soak_workload_error: {error}");
            Some("offline_workload_failed")
        }
    };
    let status = if failure_reason.is_none() {
        "passed"
    } else {
        "failed"
    };
    let summary = SoakSummary {
        schema: SUMMARY_SCHEMA,
        mode: MODE,
        seed: config.seed,
        duration_secs: config.duration_secs,
        elapsed_ms: elapsed.as_millis(),
        cycles,
        schedule_version: SCHEDULE_VERSION,
        schedule_digest,
        tracker_backend: TRACKER_BACKEND,
        counters,
        assertions,
        status,
        failure_reason,
    };
    if let Err(error) = write_summary(&config.output, &summary) {
        let _ = progress.record(elapsed, cycles, &summary.counters, "failed");
        return Err(error);
    }
    progress.record(elapsed, cycles, &summary.counters, status)?;
    Ok(summary)
}

async fn execute_workload(
    config: &SoakConfig,
    harness: &mut OfflineHarness,
    progress: &mut ProgressWriter,
    counters: &mut SoakCounters,
    cycles: &mut u64,
    started: Instant,
) -> SoakResult<()> {
    harness.run_recovery_probe(counters).await?;
    loop {
        *cycles += 1;
        harness.run_cycle(*cycles, config.seed, counters).await?;
        let elapsed = started.elapsed();
        progress.record(elapsed, *cycles, counters, "running")?;
        if config.test_fail_after_cycles == Some(*cycles) {
            return Err(std::io::Error::other("deterministic injected mid-run failure").into());
        }
        if elapsed >= config.duration() {
            return Ok(());
        }
        tokio::time::sleep(CYCLE_INTERVAL.min(config.duration().saturating_sub(elapsed))).await;
        if started.elapsed() >= config.duration() {
            return Ok(());
        }
    }
}

fn build_assertions(cycles: u64, counters: &SoakCounters, schedule_digest: &str) -> SoakAssertions {
    SoakAssertions {
        offline_only: true,
        queue_drained: counters.queue_remaining == 0
            && counters.queue_submitted == counters.queue_completed,
        outbox_drained: counters.outbox_remaining == 0
            && counters.outbox_staged == counters.outbox_delivered,
        no_duplicate_success: counters.duplicate_successes == 0,
        no_target_stall: counters.target_stalls == 0,
        failed_send_retried: counters.mirror_send_failures == 1
            && counters.mirror_send_retries == 1
            && counters.mirror_retry_same_message == 1
            && counters.mirror_send_attempts == counters.mirror_sent + 1,
        deterministic_schedule_exercised: cycles > 0
            && counters.schedule_steps == cycles
            && schedule_digest.len() == 64,
        healthy_target_progressed_while_peer_unavailable: counters.recovery_injections == 1
            && counters.recovery_attempts == 3
            && counters.recovery_successes == 1
            && counters.healthy_target_progressed == 1,
        durable_queue_backoff_exercised: counters.queue_accepted_with_warning == 1
            && counters.recovery_deferred_until_due == 1
            && counters.retry_clock_advances == 1,
        generation_reuse_recovery_exercised: counters.generation_reuse_restarts == 1,
        repeated_assistant_shapes_deduped: counters.mirror_events == cycles.saturating_mul(3)
            && counters.mirror_sent == cycles,
        disk_backed_exact_tracker: counters.tracker_cache_kib > 0
            && counters.tracker_cache_kib <= 256
            && counters.tracker_disk_rows + counters.duplicate_successes
                == counters.outbox_delivered + counters.mirror_sent,
    }
}

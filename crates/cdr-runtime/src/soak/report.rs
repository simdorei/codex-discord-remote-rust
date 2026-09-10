use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::{SoakResult, ensure_parent};

pub const SUMMARY_SCHEMA: &str = "cdr.offline-soak.summary.v1";
pub const PROGRESS_SCHEMA: &str = "cdr.offline-soak.progress.v1";
pub const MODE: &str = "offline_fake_replay";
const PROGRESS_BUCKETS: u64 = 63;

#[derive(Clone, Debug, Default, Serialize)]
pub struct SoakCounters {
    pub queue_submitted: u64,
    pub queue_completed: u64,
    pub outbox_staged: u64,
    pub outbox_delivered: u64,
    pub mirror_events: u64,
    pub mirror_send_attempts: u64,
    pub mirror_send_failures: u64,
    pub mirror_send_retries: u64,
    pub mirror_retry_same_message: u64,
    pub mirror_sent: u64,
    pub duplicate_successes: u64,
    pub target_stalls: u64,
    pub schedule_steps: u64,
    pub recovery_injections: u64,
    pub recovery_attempts: u64,
    pub recovery_successes: u64,
    pub healthy_target_progressed: u64,
    pub queue_accepted_with_warning: u64,
    pub recovery_deferred_until_due: u64,
    pub retry_clock_advances: u64,
    pub generation_reuse_restarts: u64,
    pub tracker_disk_rows: u64,
    pub tracker_cache_kib: u64,
    pub queue_remaining: u64,
    pub outbox_remaining: u64,
}

#[derive(Clone, Debug, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct SoakAssertions {
    pub offline_only: bool,
    pub queue_drained: bool,
    pub outbox_drained: bool,
    pub no_duplicate_success: bool,
    pub no_target_stall: bool,
    pub failed_send_retried: bool,
    pub deterministic_schedule_exercised: bool,
    pub healthy_target_progressed_while_peer_unavailable: bool,
    pub durable_queue_backoff_exercised: bool,
    pub generation_reuse_recovery_exercised: bool,
    pub repeated_assistant_shapes_deduped: bool,
    pub disk_backed_exact_tracker: bool,
}

impl SoakAssertions {
    #[must_use]
    pub const fn all_passed(&self) -> bool {
        self.offline_only
            && self.queue_drained
            && self.outbox_drained
            && self.no_duplicate_success
            && self.no_target_stall
            && self.failed_send_retried
            && self.deterministic_schedule_exercised
            && self.healthy_target_progressed_while_peer_unavailable
            && self.durable_queue_backoff_exercised
            && self.generation_reuse_recovery_exercised
            && self.repeated_assistant_shapes_deduped
            && self.disk_backed_exact_tracker
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SoakSummary {
    pub schema: &'static str,
    pub mode: &'static str,
    pub seed: u64,
    pub duration_secs: u64,
    pub elapsed_ms: u128,
    pub cycles: u64,
    pub schedule_version: u32,
    pub schedule_digest: String,
    pub tracker_backend: &'static str,
    pub counters: SoakCounters,
    pub assertions: SoakAssertions,
    pub status: &'static str,
    pub failure_reason: Option<&'static str>,
}

#[derive(Serialize)]
struct ProgressRecord<'a> {
    schema: &'static str,
    mode: &'static str,
    seed: u64,
    elapsed_ms: u128,
    cycle: u64,
    counters: &'a SoakCounters,
    status: &'static str,
}

pub(super) struct ProgressWriter {
    writer: BufWriter<File>,
    duration: Duration,
    seed: u64,
    last_bucket: Option<u64>,
    terminal_written: bool,
}

impl ProgressWriter {
    pub fn create(path: &Path, duration: Duration, seed: u64) -> SoakResult<Self> {
        ensure_parent(path)?;
        Ok(Self {
            writer: BufWriter::new(OpenOptions::new().write(true).create_new(true).open(path)?),
            duration,
            seed,
            last_bucket: None,
            terminal_written: false,
        })
    }

    pub fn record(
        &mut self,
        elapsed: Duration,
        cycle: u64,
        counters: &SoakCounters,
        status: &'static str,
    ) -> SoakResult<()> {
        let bucket = progress_bucket(elapsed, self.duration);
        let terminal = status != "running";
        if self.terminal_written
            || (!terminal && self.last_bucket.is_some_and(|last| bucket <= last))
        {
            return Ok(());
        }
        serde_json::to_writer(
            &mut self.writer,
            &ProgressRecord {
                schema: PROGRESS_SCHEMA,
                mode: MODE,
                seed: self.seed,
                elapsed_ms: elapsed.as_millis(),
                cycle,
                counters,
                status,
            },
        )?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if terminal {
            self.terminal_written = true;
        } else {
            self.last_bucket = Some(bucket);
        }
        Ok(())
    }
}

pub fn write_summary(path: &Path, summary: &SoakSummary) -> SoakResult<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(summary)?)?;
    file.flush()?;
    Ok(())
}

fn progress_bucket(elapsed: Duration, duration: Duration) -> u64 {
    let elapsed = elapsed.as_nanos().min(duration.as_nanos());
    let denominator = duration.as_nanos().max(1);
    u64::try_from(elapsed * u128::from(PROGRESS_BUCKETS - 1) / denominator)
        .unwrap_or(PROGRESS_BUCKETS - 1)
}

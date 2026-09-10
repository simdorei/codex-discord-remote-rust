use std::path::{Path, PathBuf};
use std::sync::Arc;

use cdr_store::delivery::{complete, list_pending};
use cdr_store::queue::list;

use crate::queue_runner::QueueCoordinator;
use crate::session_mirror_worker::SessionMirrorWorker;

use super::SoakResult;
use super::fake_backend::FakeTurnBackend;
use super::fake_sender::FakeSessionMirrorSender;
use super::fixture::{append_repeated_assistant_shapes, compact_rollout, seed_state};
use super::recovery::run_recovery_probe;
use super::report::SoakCounters;
use super::schedule::DeterministicSchedule;
use super::success_tracker::{DiskSuccessTracker, OUTBOX_NAMESPACE};

#[cfg(test)]
#[path = "repeated_cycle_contract.rs"]
mod repeated_cycle_contract;

pub(super) struct OfflineHarness {
    db: PathBuf,
    rollout: PathBuf,
    backend: Arc<FakeTurnBackend>,
    queue: QueueCoordinator<FakeTurnBackend>,
    sender: Arc<FakeSessionMirrorSender>,
    mirror: SessionMirrorWorker<FakeSessionMirrorSender>,
    success_tracker: Arc<DiskSuccessTracker>,
    schedule: DeterministicSchedule,
}

impl OfflineHarness {
    pub fn create(root: &Path, seed: u64) -> SoakResult<Self> {
        let db = root.join("mirror.sqlite");
        let state_db = root.join("state.sqlite");
        let rollout = root.join("rollout.jsonl");
        seed_state(&state_db, &db, &rollout)?;
        let backend = Arc::new(FakeTurnBackend::default());
        let success_tracker = Arc::new(DiskSuccessTracker::create(
            &root.join("success-tracker.sqlite"),
        )?);
        let sender = Arc::new(FakeSessionMirrorSender::new(Arc::clone(&success_tracker)));
        sender.schedule_one_failure();
        Ok(Self {
            queue: QueueCoordinator::new(db.clone(), Arc::clone(&backend)),
            mirror: SessionMirrorWorker::new(state_db, db.clone(), Arc::clone(&sender)),
            db,
            rollout,
            backend,
            sender,
            success_tracker,
            schedule: DeterministicSchedule::new(seed),
        })
    }

    pub async fn run_recovery_probe(&mut self, counters: &mut SoakCounters) -> SoakResult<()> {
        run_recovery_probe(&self.db, &self.queue, &self.backend, counters).await?;
        self.drain_outbox(counters)?;
        self.capture_state(counters)
    }

    pub async fn run_cycle(
        &mut self,
        cycle: u64,
        seed: u64,
        counters: &mut SoakCounters,
    ) -> SoakResult<()> {
        let order = self.schedule.target_order();
        counters.schedule_steps += 1;
        let mut mirror_origin_turn = None;
        for target in order {
            self.submit_pair(target, cycle, counters).await?;
        }
        for target in order {
            let last_turn = self.complete_pair(target, cycle, counters).await?;
            if target == "thread-a" {
                mirror_origin_turn = Some(last_turn);
            }
        }
        self.drain_outbox(counters)?;
        append_repeated_assistant_shapes(
            &self.rollout,
            cycle,
            seed,
            mirror_origin_turn.as_deref().unwrap_or("missing-origin"),
        )?;
        self.poll_mirror(counters).await?;
        compact_rollout(&self.rollout, &self.db, cycle)?;
        self.capture_state(counters)?;
        Ok(())
    }

    pub fn schedule_digest(&self) -> String {
        self.schedule.digest()
    }

    async fn submit_pair(
        &self,
        target: &str,
        cycle: u64,
        counters: &mut SoakCounters,
    ) -> SoakResult<()> {
        let target_offset = u64::from(target == "thread-b");
        for ordinal in 0..2_u64 {
            let message_id = cycle
                .saturating_mul(10)
                .saturating_add(target_offset * 2 + ordinal + 1);
            let prompt = format!("offline:{cycle}:{target}:{ordinal}");
            self.queue
                .submit(target, 200 + target_offset, 300, Some(message_id), &prompt)
                .await?;
            counters.queue_submitted += 1;
        }
        Ok(())
    }

    async fn complete_pair(
        &self,
        target: &str,
        cycle: u64,
        counters: &mut SoakCounters,
    ) -> SoakResult<String> {
        let expected_starts = self.backend.starts(target).await + 1;
        let mut last_turn = String::new();
        for ordinal in 0..2_u64 {
            let turn_id = self.backend.finish_active(target).await?;
            let logical_id = format!("queue:{cycle}:{target}:{ordinal}");
            let staged = self
                .queue
                .stage_turn_completion(target, &turn_id, &logical_id)
                .await?;
            if staged.is_some() {
                counters.queue_completed += 1;
                counters.outbox_staged += 1;
            } else {
                counters.target_stalls += 1;
            }
            last_turn = turn_id;
        }
        if self.backend.starts(target).await != expected_starts {
            counters.target_stalls += 1;
        }
        Ok(last_turn)
    }

    fn drain_outbox(&mut self, counters: &mut SoakCounters) -> SoakResult<()> {
        for delivery in list_pending(&self.db)? {
            let logical_id = delivery.content;
            if complete(&self.db, &delivery.delivery_id)? {
                let _ = self.success_tracker.record(OUTBOX_NAMESPACE, &logical_id)?;
                counters.outbox_delivered += 1;
            }
        }
        Ok(())
    }

    async fn poll_mirror(&self, counters: &mut SoakCounters) -> SoakResult<()> {
        let poll = match self.mirror.poll_once().await {
            Ok(poll) => poll,
            Err(error) if error.is_delivery_failure() && counters.mirror_send_retries == 0 => {
                counters.mirror_send_retries += 1;
                self.mirror.poll_once().await?
            }
            Err(error) => return Err(error.into()),
        };
        counters.mirror_events += u64::try_from(poll.events)?;
        let metrics = self.sender.metrics();
        counters.mirror_send_attempts = metrics.attempts;
        counters.mirror_send_failures = metrics.failures;
        counters.mirror_retry_same_message = metrics.matching_retries;
        counters.mirror_sent = metrics.successes;
        counters.duplicate_successes =
            self.success_tracker.duplicate_count(OUTBOX_NAMESPACE)? + metrics.duplicate_successes;
        Ok(())
    }

    fn capture_state(&self, counters: &mut SoakCounters) -> SoakResult<()> {
        counters.queue_remaining = u64::try_from(list(&self.db)?.len())?;
        counters.outbox_remaining = u64::try_from(list_pending(&self.db)?.len())?;
        counters.tracker_disk_rows = self.success_tracker.cardinality()?;
        counters.tracker_cache_kib = self.success_tracker.cache_kib()?;
        if counters.queue_remaining > 0 || counters.outbox_remaining > 0 {
            counters.target_stalls += 1;
        }
        Ok(())
    }
}

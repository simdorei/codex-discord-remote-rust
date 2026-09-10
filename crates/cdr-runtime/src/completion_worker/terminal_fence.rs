use std::collections::BTreeSet;
use std::sync::Mutex;
use tokio::sync::watch;

pub(super) struct TerminalFence {
    stopped: Mutex<BTreeSet<(u64, String, String)>>,
    changed: watch::Sender<u64>,
}
impl Default for TerminalFence {
    fn default() -> Self {
        Self {
            stopped: Mutex::default(),
            changed: watch::channel(0).0,
        }
    }
}
impl TerminalFence {
    pub fn stop(&self, generation: u64, thread: &str, turn: &str) {
        let Ok(mut stopped) = self.stopped.lock() else {
            eprintln!("typing_terminal_fence_poisoned; typing disabled");
            self.changed
                .send_modify(|version| *version = version.wrapping_add(1));
            return;
        };
        stopped.insert((generation, thread.into(), turn.into()));
        self.changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
    pub fn stopped(&self, generation: u64, thread: &str, turn: &str) -> bool {
        self.stopped.lock().map_or(true, |stopped| {
            stopped.contains(&(generation, thread.into(), turn.into()))
        })
    }
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }
    #[cfg(test)]
    pub fn pending_subscribers(&self) -> usize {
        self.changed.receiver_count()
    }
    pub fn retention_version(&self) -> u64 {
        *self.changed.borrow()
    }
    pub fn retain(
        &self,
        generation: u64,
        jobs: &[cdr_store::queue::StoredQueueJob],
        observed_version: u64,
    ) {
        if let Ok(mut stopped) = self.stopped.lock() {
            // Compare under the same lock used to publish terminal and version.
            // A queue snapshot predating any terminal cannot authorize pruning.
            if *self.changed.borrow() != observed_version {
                return;
            }
            stopped.retain(|(g, thread, turn)| {
                *g == generation
                    && jobs.iter().any(|job| {
                        job.target_thread_id == *thread && job.turn_id.as_deref() == Some(turn)
                    })
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_queue_snapshot_cannot_erase_a_new_terminal_observation() {
        let fence = TerminalFence::default();
        let snapshot_version = fence.retention_version();
        // A queue read began before the terminal; its result omits the job.
        fence.stop(1, "thread", "turn");
        fence.retain(1, &[], snapshot_version);
        assert!(fence.stopped(1, "thread", "turn"));
    }
    #[tokio::test]
    async fn terminal_revokes_pending_typing_before_any_disk_operation() {
        let fence = TerminalFence::default();
        let mut changed = fence.subscribe();
        assert!(!fence.stopped(1, "thread", "turn"));
        fence.stop(1, "thread", "turn");
        changed.changed().await.unwrap();
        assert!(fence.stopped(1, "thread", "turn"));
        assert!(!fence.stopped(1, "thread", "next"));
        assert!(!fence.stopped(2, "thread", "turn"));
    }
}

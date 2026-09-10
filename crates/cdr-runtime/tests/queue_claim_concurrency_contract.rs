use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cdr_runtime::queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord};
use cdr_store::queue::{NewQueueJob, QueueJobState, enqueue, list};
use tokio::sync::Barrier;

struct RacingBackend {
    active_checks: Barrier,
    starts: AtomicUsize,
}

impl RacingBackend {
    fn new() -> Self {
        Self {
            active_checks: Barrier::new(2),
            starts: AtomicUsize::new(0),
        }
    }
}

impl TurnBackend for RacingBackend {
    fn generation(&self) -> u64 {
        1
    }

    fn active_turn_id<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async move {
            self.active_checks.wait().await;
            Ok(None)
        })
    }

    fn read_turns<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resume_thread<'a>(&'a self, _thread_id: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn start_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            let ordinal = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(format!("turn-{ordinal}"))
        })
    }
}

#[tokio::test]
async fn two_coordinators_claim_one_pending_job_exactly_once() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("queue.sqlite");
    enqueue(
        &db,
        NewQueueJob {
            job_id: "one-job",
            target_thread_id: "one-target",
            channel_id: 10,
            owner_user_id: Some(11),
            discord_message_id: Some(12),
            app_server_generation: 1,
            prompt: "run once",
            queued: true,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    let backend = Arc::new(RacingBackend::new());
    let first = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
    let second = QueueCoordinator::new(db.clone(), Arc::clone(&backend));

    let (first_report, second_report) = tokio::join!(first.recover(), second.recover());

    let first_report = first_report.unwrap();
    let second_report = second_report.unwrap();
    assert_eq!(first_report.started + second_report.started, 1);
    assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
    let jobs = list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, QueueJobState::Running);
    assert_eq!(jobs[0].attempt_count, 1);
}

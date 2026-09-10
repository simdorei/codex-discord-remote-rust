use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use cdr_store::mapping::upsert_thread;
use cdr_store::queue::{NewQueueJob, begin_attempt, enqueue, mark_running};

use super::*;

struct FakeServer {
    lifecycle: Mutex<VecDeque<ResidentLifecycleSnapshot>>,
    active: BTreeMap<String, String>,
    unsettled: bool,
}

impl FakeServer {
    fn new(lifecycle: Vec<ResidentLifecycleSnapshot>) -> Self {
        Self {
            lifecycle: Mutex::new(lifecycle.into()),
            active: BTreeMap::new(),
            unsettled: false,
        }
    }
}

impl LiveDrainServer for FakeServer {
    async fn lifecycle(&self) -> ResidentLifecycleSnapshot {
        let mut snapshots = self.lifecycle.lock().unwrap();
        if snapshots.len() > 1 {
            snapshots.pop_front().unwrap()
        } else {
            snapshots.front().unwrap().clone()
        }
    }

    async fn active_turn(&self, thread_id: &str) -> Result<Option<String>, AppServerError> {
        Ok(self.active.get(thread_id).cloned())
    }

    async fn has_unsettled_requests(&self) -> Result<bool, AppServerError> {
        Ok(self.unsettled)
    }
}

fn healthy(generation: u64) -> ResidentLifecycleSnapshot {
    ResidentLifecycleSnapshot {
        generation,
        healthy: true,
        quarantined: false,
        restart_pending: false,
        process_id: Some(100),
    }
}

fn seed_target(path: &Path) {
    upsert_thread(path, "thread-a", "project", "A", 10, 20, 1.0).unwrap();
}

#[tokio::test]
async fn idle_live_server_and_stable_durable_snapshot_are_ready() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);

    assert_eq!(
        check_runtime_quiescence(&db, &FakeServer::new(vec![healthy(1)]))
            .await
            .unwrap(),
        LiveDrainState::Ready
    );
}

#[tokio::test]
async fn active_turn_or_unsettled_request_blocks_live_drain() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);
    let mut active = FakeServer::new(vec![healthy(1)]);
    active.active.insert("thread-a".into(), "turn-a".into());
    let mut unsettled = FakeServer::new(vec![healthy(1)]);
    unsettled.unsettled = true;

    for (server, expected) in [(active, "active turn"), (unsettled, "unsettled")] {
        let state = check_runtime_quiescence(&db, &server).await.unwrap();
        assert!(
            matches!(state, LiveDrainState::Blocked { ref reason } if reason.contains(expected)),
            "expected {expected}: {state:?}"
        );
    }
}

#[tokio::test]
async fn lifecycle_change_between_observations_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);
    let server = FakeServer::new(vec![healthy(1), healthy(2)]);

    let state = check_runtime_quiescence(&db, &server).await.unwrap();
    assert!(matches!(
        state,
        LiveDrainState::Blocked { ref reason } if reason.contains("changed")
    ));
}

#[tokio::test]
async fn durable_running_work_blocks_before_live_server_is_considered_idle() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    seed_target(&db);
    enqueue(
        &db,
        NewQueueJob {
            job_id: "job-a",
            target_thread_id: "thread-a",
            channel_id: 20,
            owner_user_id: Some(1),
            discord_message_id: Some(2),
            app_server_generation: 1,
            prompt: "work",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    begin_attempt(&db, "job-a", &[], 1).unwrap();
    mark_running(&db, "job-a", "turn-a", 1).unwrap();

    let state = check_runtime_quiescence(&db, &FakeServer::new(vec![healthy(1)]))
        .await
        .unwrap();
    assert!(matches!(
        state,
        LiveDrainState::Blocked { ref reason } if reason.contains("running")
    ));
}

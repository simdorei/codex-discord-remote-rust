//! PATCH-04B: cancellation committed before the writer must suppress the old start.
use std::{path::PathBuf, sync::Arc, time::Duration};

use cdr_app_server::ResidentAppServer;
use cdr_runtime::{
    app_backend::AppServerTurnBackend,
    dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
    soak::native_fixture,
};
use cdr_store::queue::{NewQueueJob, cancel_for_recovery, enqueue, list_filtered};
use tokio::sync::Semaphore;

#[path = "support/action_app_server.rs"]
mod server_support;

struct PausedStart {
    inner: AppServerTurnBackend,
    reached: Semaphore,
    release: Semaphore,
}

impl TurnBackend for PausedStart {
    fn start_claimed_turn<'a>(
        &'a self,
        claimed: &'a cdr_store::queue::StoredQueueJob,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            if claimed.target_thread_id == "recover-a" {
                self.reached.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            self.inner.start_claimed_turn(claimed).await
        })
    }

    fn generation(&self) -> u64 {
        self.inner.generation()
    }
    fn resident_instance_id(&self) -> Option<&str> {
        self.inner.resident_instance_id()
    }
    fn active_turn_id<'a>(&'a self, thread: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        self.inner.active_turn_id(thread)
    }
    fn read_turns<'a>(&'a self, thread: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        self.inner.read_turns(thread)
    }
    fn resume_thread<'a>(&'a self, thread: &'a str) -> BoxBackendFuture<'a, ()> {
        self.inner.resume_thread(thread)
    }
    fn start_turn<'a>(&'a self, thread: &'a str, prompt: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            if thread == "recover-a" {
                self.reached.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            self.inner.start_turn(thread, prompt).await
        })
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: Arc<ResidentAppServer>,
    backend: Arc<PausedStart>,
    queue: Arc<QueueCoordinator<PausedStart>>,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let durable = Arc::new(
            RuntimeDeadGenerationFence::new(db.clone(), "p04-recover-race".into(), None).unwrap(),
        );
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, durable)
                .await
                .unwrap(),
        );
        let backend = Arc::new(PausedStart {
            inner: AppServerTurnBackend::new(Arc::clone(&server)),
            reached: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        let queue = Arc::new(QueueCoordinator::new(db.clone(), Arc::clone(&backend)));
        enqueue(
            &db,
            NewQueueJob {
                job_id: "job-a",
                target_thread_id: "recover-a",
                channel_id: 99,
                owner_user_id: Some(20),
                discord_message_id: Some(101),
                app_server_generation: i64::try_from(server.generation()).unwrap(),
                prompt: "original A must not dispatch after recovery cancellation",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        Self {
            _temp: temp,
            db,
            log,
            server,
            backend,
            queue,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p04_recover_before_actual_writer_prevents_old_start_and_keeps_b_live() {
    let fixture = Fixture::new().await;
    let queue = Arc::clone(&fixture.queue);
    let starting = tokio::spawn(async move { queue.kick_target("recover-a").await });
    tokio::time::timeout(Duration::from_secs(10), fixture.backend.reached.acquire())
        .await
        .expect("queue reached actual backend start")
        .unwrap()
        .forget();
    let before = list_filtered(&fixture.db, Some("recover-a"), None).unwrap();
    assert_eq!(before[0].state, cdr_store::queue::QueueJobState::Starting);
    let cancelled = cancel_for_recovery(&fixture.db, "recover-a", 99, 20, 2.0).unwrap();
    fixture.backend.release.add_permits(1);
    let outcome = tokio::time::timeout(Duration::from_secs(10), starting).await;
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        fixture
            .queue
            .submit("thread-b", 99, 20, None, "independent B"),
    )
    .await;
    let cold = QueueCoordinator::new(fixture.db.clone(), Arc::clone(&fixture.backend));
    let cold_result =
        tokio::time::timeout(Duration::from_secs(5), cold.kick_target("recover-a")).await;
    fixture.server.close().await.unwrap();
    let calls = server_support::rpc_log(&fixture.log);
    assert_eq!(cancelled.jobs, ["job-a"]);
    assert_eq!(cancelled.started_or_uncertain, 1);
    assert!(
        outcome.unwrap().unwrap().is_err(),
        "cancelled A must lose the claimed attempt"
    );
    assert!(
        b.unwrap().unwrap().turn_id.is_some(),
        "independent B must remain live"
    );
    assert!(cold_result.unwrap().is_ok());
    assert!(
        list_filtered(&fixture.db, Some("recover-a"), None)
            .unwrap()
            .is_empty()
    );
    assert!(
        cdr_store::execution_hold::reason(&fixture.db, "job-a")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "turn/start" && v["params"]["threadId"] == "recover-a")
            .count(),
        0,
        "a committed recovery cancellation must win before the old writer claim"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "turn/start" && v["params"]["threadId"] == "thread-b")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p04_intact_original_claim_starts_once() {
    let fixture = Fixture::new().await;
    fixture.backend.release.add_permits(1);
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        fixture.queue.kick_target("recover-a"),
    )
    .await;
    fixture.server.close().await.unwrap();
    outcome.unwrap().unwrap();
    let jobs = list_filtered(&fixture.db, Some("recover-a"), None).unwrap();
    assert_eq!(jobs[0].state, cdr_store::queue::QueueJobState::Running);
    assert_eq!(jobs[0].attempt_count, 1);
    assert_eq!(
        server_support::rpc_log(&fixture.log)
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p04_claim_commit_failure_keeps_starting_and_never_dispatches() {
    let fixture = Fixture::new().await;
    rusqlite::Connection::open(&fixture.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_queue_dispatch BEFORE INSERT ON codex_mutation_attempts
         WHEN NEW.method='turn/start'
         BEGIN SELECT RAISE(ABORT,'injected queue claim write failure'); END;",
        )
        .unwrap();
    fixture.backend.release.add_permits(1);
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        fixture.queue.kick_target("recover-a"),
    )
    .await;
    let cold = QueueCoordinator::new(fixture.db.clone(), Arc::clone(&fixture.backend));
    let cold_outcome =
        tokio::time::timeout(Duration::from_secs(5), cold.kick_target("recover-a")).await;
    fixture.server.close().await.unwrap();
    assert!(outcome.unwrap().is_err());
    cold_outcome.unwrap().unwrap();
    let jobs = list_filtered(&fixture.db, Some("recover-a"), None).unwrap();
    assert_eq!(jobs[0].state, cdr_store::queue::QueueJobState::Starting);
    assert_eq!(jobs[0].attempt_count, 1);
    assert_eq!(jobs[0].execution_generation, Some(1));
    assert!(jobs[0].turn_id.is_none());
    assert!(jobs[0].baseline_turn_ids.is_empty());
    assert_eq!(
        jobs[0].prompt,
        "original A must not dispatch after recovery cancellation"
    );
    assert!(
        !server_support::rpc_log(&fixture.log)
            .iter()
            .any(|v| v["method"] == "turn/start")
    );
}

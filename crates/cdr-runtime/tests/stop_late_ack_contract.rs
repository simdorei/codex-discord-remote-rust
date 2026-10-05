use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_runtime::{
    action_executor::ActionExecutor,
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
    soak::native_fixture,
};
use cdr_store::{
    ingress::stop::control,
    queue::{self, QueueJobState, StoredQueueJob},
};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[path = "support/action_app_server.rs"]
mod server_support;

struct DelayedAck {
    inner: AppServerTurnBackend,
    reached: Semaphore,
    release: Semaphore,
}
impl TurnBackend for DelayedAck {
    fn generation(&self) -> u64 {
        self.inner.generation()
    }
    fn resident_instance_id(&self) -> Option<&str> {
        self.inner.resident_instance_id()
    }
    fn active_turn_id<'a>(&'a self, target: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        self.inner.active_turn_id(target)
    }
    fn read_turns<'a>(&'a self, target: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        self.inner.read_turns(target)
    }
    fn resume_thread<'a>(&'a self, target: &'a str) -> BoxBackendFuture<'a, ()> {
        self.inner.resume_thread(target)
    }
    fn start_turn<'a>(&'a self, target: &'a str, prompt: &'a str) -> BoxBackendFuture<'a, String> {
        self.inner.start_turn(target, prompt)
    }
    fn start_claimed_turn<'a>(
        &'a self,
        claimed: &'a StoredQueueJob,
    ) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            let turn = self.inner.start_claimed_turn(claimed).await?;
            if claimed.target_thread_id == "thread-b" {
                self.reached.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            Ok(turn)
        })
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: Arc<ResidentAppServer>,
    backend: Arc<DelayedAck>,
    queue: Arc<QueueCoordinator<DelayedAck>>,
    executor: ActionExecutor<DelayedAck>,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        let log = temp.path().join("rpc.jsonl");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let fence = Arc::new(
            RuntimeDeadGenerationFence::new(db.clone(), "late-ack-stop".into(), None).unwrap(),
        );
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        config.environment.insert(
            "CDR_STOP_NEXT_ACTIVE_PATH".into(),
            log.with_file_name("next-active.json")
                .to_string_lossy()
                .into_owned(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, fence)
                .await
                .unwrap(),
        );
        let backend = Arc::new(DelayedAck {
            inner: AppServerTurnBackend::new(server.clone()),
            reached: Semaphore::new(0),
            release: Semaphore::new(0),
        });
        let queue = Arc::new(QueueCoordinator::new(db.clone(), backend.clone()));
        let executor = ActionExecutor::new(
            state,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            queue.clone(),
        )
        .with_server(server.clone());
        Self {
            _temp: temp,
            db,
            log,
            server,
            backend,
            queue,
            executor,
        }
    }
    fn original(&self) -> StoredQueueJob {
        queue::list_filtered(&self.db, Some("thread-b"), None)
            .unwrap()
            .remove(0)
    }
    fn count(&self, method: &str, target: &str) -> usize {
        server_support::rpc_log(&self.log)
            .iter()
            .filter(|r| r["method"] == method && r["params"]["threadId"] == target)
            .count()
    }
    async fn observe_original(&self, turn: &str) {
        std::fs::write(
            self.log.with_file_name("next-active.json"),
            serde_json::to_vec(&json!({"threadId":"thread-b","turnId":turn})).unwrap(),
        )
        .unwrap();
        self.server
            .execute(
                AppRequest {
                    method: "thread/read",
                    params: json!({"threadId":"thread-b"}),
                    timeout: Duration::from_secs(2),
                },
                Some(self.server.generation()),
            )
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_original_start_ack_binds_one_interrupt_while_native_b_progresses() {
    let f = Fixture::new().await;
    let q = f.queue.clone();
    let task =
        tokio::spawn(async move { q.submit("thread-b", 99, 20, Some(901), "original A").await });
    tokio::time::timeout(Duration::from_secs(5), f.backend.reached.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let starting = f.original();
    assert_eq!(starting.state, QueueJobState::Starting);
    let stopped = tokio::time::timeout(
        Duration::from_secs(3),
        f.executor.execute(
            CommandAction::Stop {
                reference: Some("thread-b".into()),
            },
            99,
            20,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(stopped.text.contains("Execution end is not confirmed"));
    assert_eq!(f.original(), starting);
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    // A still owns its coordinator target lock and cannot finish without release.
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        f.queue
            .submit("independent-b", 100, 21, Some(902), "independent B"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(b.turn_id.is_some());
    assert!(!task.is_finished());
    f.backend.release.add_permits(1);
    let outcome = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let running = f.original();
    let controls = control::pending_after(&f.db, 0).unwrap();
    if controls.len() != 1 {
        f.server.close().await.unwrap();
        assert_eq!(
            controls.len(),
            1,
            "late original ACK must bind its durable stop, not lose the accepted intent"
        );
    }
    let original = &controls[0].1;
    assert_eq!(original.resident, f.server.instance_id());
    assert_eq!(original.generation, starting.app_server_generation);
    assert_eq!(original.turn, outcome.turn_id.clone().unwrap());
    assert_original_ack(&f.db, &starting, &running);
    f.observe_original(&original.turn).await;
    let mut cursor = 0;
    let dispatched = tokio::time::timeout(
        Duration::from_secs(5),
        f.executor.process_stop_controls(&mut cursor),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(dispatched, 1);
    assert_eq!(
        control::phase(&f.db, &original.operation_id)
            .unwrap()
            .as_deref(),
        Some("acknowledged")
    );
    assert!(
        control::target_is_held(&f.db, "thread-b").unwrap(),
        "interrupt ACK is not execution end"
    );
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        0
    );
    f.queue.kick_target("thread-b").await.unwrap();
    assert_eq!(f.original(), running);
    assert_eq!(f.count("turn/start", "thread-b"), 1);
    assert_eq!(f.count("turn/start", "independent-b"), 1);
    assert_eq!(f.count("turn/interrupt", "thread-b"), 1);
    let frames = server_support::rpc_log(&f.log);
    assert!(
        frames
            .iter()
            .filter(|r| r["method"] == "turn/interrupt")
            .all(
                |r| r["params"]["threadId"] == "thread-b" && r["params"]["turnId"] == original.turn
            )
    );
    f.server.close().await.unwrap();
}

fn assert_original_ack(db: &std::path::Path, starting: &StoredQueueJob, running: &StoredQueueJob) {
    assert_eq!(running.prompt, starting.prompt);
    assert_eq!(running.attempt_count, starting.attempt_count);
    assert_eq!(running.baseline_turn_ids, starting.baseline_turn_ids);
    assert_eq!(running.execution_generation, starting.execution_generation);
    assert!(
        cdr_store::execution_hold::reason(db, &starting.job_id)
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn late_control_store_failure_keeps_starting_across_cold_recovery_without_replay() {
    let f = Fixture::new().await;
    let q = f.queue.clone();
    let task =
        tokio::spawn(async move { q.submit("thread-b", 99, 20, Some(911), "original A").await });
    tokio::time::timeout(Duration::from_secs(5), f.backend.reached.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let starting = f.original();
    let stop = tokio::time::timeout(
        Duration::from_secs(3),
        f.executor.execute(
            CommandAction::Stop {
                reference: Some("thread-b".into()),
            },
            99,
            20,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(stop.text.contains("Stop accepted"));
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_late_control BEFORE INSERT ON cdr_stop_controls
         BEGIN SELECT RAISE(ABORT,'injected late control persistence failure'); END;",
        )
        .unwrap();
    f.backend.release.add_permits(1);
    let outcome = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(outcome.is_err());
    assert_eq!(
        f.original(),
        starting,
        "atomic ACK rollback preserves the original Starting"
    );
    assert!(control::pending_after(&f.db, 0).unwrap().is_empty());
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch("DROP TRIGGER fail_late_control;")
        .unwrap();
    let cold = QueueCoordinator::new(f.db.clone(), f.backend.clone());
    tokio::time::timeout(Duration::from_secs(5), cold.kick_target("thread-b"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.original(), starting);
    assert_eq!(f.count("turn/start", "thread-b"), 1);
    assert_eq!(f.count("turn/interrupt", "thread-b"), 0);
    assert!(
        cdr_store::execution_hold::reason(&f.db, &starting.job_id)
            .unwrap()
            .is_some()
    );
    f.server.close().await.unwrap();
}

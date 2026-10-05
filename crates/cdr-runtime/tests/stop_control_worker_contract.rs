use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::{
    ingress::stop::control,
    queue::{NewQueueJob, begin_attempt, enqueue, mark_running},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[path = "support/action_app_server.rs"]
mod server_support;

struct Fixture {
    temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    state: PathBuf,
    server: Arc<ResidentAppServer>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    executor: Arc<ActionExecutor<AppServerTurnBackend>>,
}
impl Fixture {
    async fn new(drop_reply: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let server = start_server(&db, &log, drop_reply, "runtime-a").await;
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        ));
        let executor = Arc::new(
            ActionExecutor::new(
                state.clone(),
                db.clone(),
                Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
                Arc::clone(&queue),
            )
            .with_server(Arc::clone(&server)),
        );
        let f = Self {
            temp,
            db,
            log,
            state,
            server,
            queue,
            executor,
        };
        f.add_running("running-a", "thread-a", "owned-a", 99, 20)
            .await;
        f
    }
    async fn add_running(&self, job: &str, target: &str, turn: &str, channel: i64, owner: i64) {
        let generation = i64::try_from(self.server.generation()).unwrap();
        enqueue(
            &self.db,
            NewQueueJob {
                job_id: job,
                target_thread_id: target,
                channel_id: channel,
                owner_user_id: Some(owner),
                discord_message_id: None,
                app_server_generation: generation,
                prompt: "original input",
                queued: true,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        begin_attempt(&self.db, job, &[], generation).unwrap();
        mark_running(&self.db, job, turn, generation).unwrap();
        self.active(target, turn).await;
    }
    async fn active(&self, target: &str, turn: &str) {
        std::fs::write(
            self.log.with_file_name("next-active.json"),
            serde_json::to_vec(&json!({"threadId":target,"turnId":turn})).unwrap(),
        )
        .unwrap();
        self.server
            .execute(
                AppRequest {
                    method: "thread/read",
                    params: json!({"threadId":target}),
                    timeout: Duration::from_secs(2),
                },
                None,
            )
            .await
            .unwrap();
    }
    async fn stop(&self, target: &str, channel: u64, owner: u64) -> control::StopControl {
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            self.executor.execute(
                CommandAction::Stop {
                    reference: Some(target.into()),
                },
                channel,
                owner,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.text.contains("Stop accepted"));
        assert!(response.text.contains("Execution end is not confirmed"));
        assert!(!response.waits_for_final);
        control::pending_after(&self.db, 0)
            .unwrap()
            .into_iter()
            .find(|(_, value)| value.target == target)
            .unwrap()
            .1
    }
    fn calls(&self, method: &str, target: &str) -> usize {
        server_support::rpc_log(&self.log)
            .iter()
            .filter(|call| call["method"] == method && call["params"]["threadId"] == target)
            .count()
    }
    fn snapshot(&self) -> Value {
        serde_json::to_value(
            cdr_store::queue::list_filtered(&self.db, Some("thread-a"), None).unwrap(),
        )
        .unwrap()
    }
}
async fn start_server(
    db: &std::path::Path,
    log: &std::path::Path,
    drop_reply: bool,
    runtime: &str,
) -> Arc<ResidentAppServer> {
    let fence = Arc::new(RuntimeDeadGenerationFence::new(db.into(), runtime.into(), None).unwrap());
    let mut config = native_fixture::config("action");
    config.environment.insert(
        "CDR_STOP_NEXT_ACTIVE_PATH".into(),
        log.with_file_name("next-active.json")
            .to_string_lossy()
            .into_owned(),
    );
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    if drop_reply {
        config
            .environment
            .insert("CDR_STOP_DROP_INTERRUPT_REPLY".into(), "1".into());
    }
    Arc::new(
        ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_worker_interrupts_once_without_claiming_ack_is_execution_end() {
    let f = Fixture::new(false).await;
    let before = f.snapshot();
    let original = f.stop("thread-a", 99, 20).await;
    let (shutdown, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(Arc::clone(&f.executor).run_stop_worker(rx));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if control::phase(&f.db, &original.operation_id)
                .unwrap()
                .as_deref()
                == Some("acknowledged")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    let mut cursor = 0;
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        0
    );
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
    assert_eq!(f.snapshot(), before);
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 1);
    assert_eq!(f.calls("turn/start", "thread-a"), 0);
    assert_eq!(f.calls("thread/resume", "thread-a"), 0);
    f.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flushed_interrupt_timeout_is_not_replayed_and_independent_b_starts() {
    let f = Fixture::new(true).await;
    let original = f.stop("thread-a", 99, 20).await;
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
        Some("unknown")
    );
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
    assert!(!f.server.lifecycle_snapshot().await.quarantined);
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        0
    );
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        f.queue.submit("thread-b", 100, 21, None, "B"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(b.turn_id.is_some());
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 1);
    assert_eq!(f.calls("turn/start", "thread-b"), 1);
    f.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_active_turn_is_never_substituted_for_original_stop_scope() {
    let f = Fixture::new(false).await;
    let original = f.stop("thread-a", 99, 20).await;
    f.active("thread-a", "later-turn").await;
    let mut cursor = 0;
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        0
    );
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 0);
    assert_eq!(
        control::phase(&f.db, &original.operation_id)
            .unwrap()
            .as_deref(),
        Some("accepted")
    );
    f.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_wire_evidence_failure_sends_nothing_and_retains_original_request() {
    let f = Fixture::new(false).await;
    let before = f.snapshot();
    let original = f.stop("thread-a", 99, 20).await;
    rusqlite::Connection::open(&f.db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_stop_wire BEFORE UPDATE OF wire_attempt ON cdr_stop_controls
         BEGIN SELECT RAISE(ABORT,'injected stop wire failure'); END;",
        )
        .unwrap();
    let mut cursor = 0;
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        1
    );
    assert_eq!(
        control::phase(&f.db, &original.operation_id)
            .unwrap()
            .as_deref(),
        Some("unknown")
    );
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 0);
    assert_eq!(f.snapshot(), before);
    assert_eq!(
        f.executor.process_stop_controls(&mut cursor).await.unwrap(),
        0
    );
    f.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_original_target_does_not_starve_another_accepted_stop() {
    let f = Fixture::new(false).await;
    f.add_running("running-b", "thread-b", "owned-b", 100, 21)
        .await;
    f.stop("thread-a", 99, 20).await;
    f.stop("thread-b", 100, 21).await;
    let _guard = f.executor.control_lock("thread-a").await.unwrap();
    let mut cursor = 0;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(5),
            f.executor.process_stop_controls(&mut cursor)
        )
        .await
        .unwrap()
        .unwrap(),
        1
    );
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 0);
    assert_eq!(f.calls("turn/interrupt", "thread-b"), 1);
    f.server.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cold_new_resident_never_adopts_an_old_accepted_interrupt() {
    let f = Fixture::new(false).await;
    let original = f.stop("thread-a", 99, 20).await;
    f.server.close().await.unwrap();
    let server = start_server(&f.db, &f.log, false, "runtime-b").await;
    assert_ne!(server.instance_id(), original.resident);
    let queue = Arc::new(QueueCoordinator::new(
        f.db.clone(),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    ));
    let cold = ActionExecutor::new(
        f.state.clone(),
        f.db.clone(),
        Arc::new(BridgeState::new(f.temp.path().join("bridge.json"))),
        queue,
    )
    .with_server(Arc::clone(&server));
    std::fs::write(
        f.log.with_file_name("next-active.json"),
        serde_json::to_vec(&json!({"threadId":"thread-a","turnId":"owned-a"})).unwrap(),
    )
    .unwrap();
    server
        .execute(
            AppRequest {
                method: "thread/read",
                params: json!({"threadId":"thread-a"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(cold.process_stop_controls(&mut 0).await.unwrap(), 0);
    assert_eq!(f.calls("turn/interrupt", "thread-a"), 0);
    assert!(control::target_is_held(&f.db, "thread-a").unwrap());
    server.close().await.unwrap();
}

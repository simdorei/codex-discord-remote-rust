use cdr_app_server::ResidentAppServer;
use cdr_runtime::{
    action_executor::{ActionContext, ActionExecutor},
    bridge_state::BridgeState,
    command_plan::CommandAction,
    dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::{BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord},
    soak::native_fixture,
};
use cdr_store::{
    ingress::stop::{control, revision},
    queue::{self, NewQueueJob},
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;

struct Backend {
    generation: u64,
    starts: Mutex<Vec<(String, String)>>,
}
impl TurnBackend for Backend {
    fn generation(&self) -> u64 {
        self.generation
    }
    fn active_turn_id<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn read_turns<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn resume_thread<'a>(&'a self, _thread: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn start_turn<'a>(&'a self, thread: &'a str, prompt: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts
                .lock()
                .await
                .push((thread.into(), prompt.into()));
            Ok(format!("turn-{thread}"))
        })
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    backend: Arc<Backend>,
    server: Option<Arc<ResidentAppServer>>,
    executor: ActionExecutor<Backend>,
}
impl Fixture {
    async fn new(resident: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        let log = temp.path().join("rpc.jsonl");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "A", 100, 42, 1.0).unwrap();
        cdr_store::mapping::upsert_thread(&db, "thread-a", "project", "B", 100, 43, 1.0).unwrap();
        let server = if resident {
            let fence = Arc::new(
                RuntimeDeadGenerationFence::new(db.clone(), "unresolved-stop".into(), None)
                    .unwrap(),
            );
            let mut config = native_fixture::config("action");
            config.environment.insert(
                "CDR_ACTION_RPC_LOG".into(),
                log.to_string_lossy().into_owned(),
            );
            Some(Arc::new(
                ResidentAppServer::start_with_dead_generation_fence(config, fence)
                    .await
                    .unwrap(),
            ))
        } else {
            None
        };
        let backend = Arc::new(Backend {
            generation: server.as_ref().map_or(7, |server| server.generation()),
            starts: Mutex::new(Vec::new()),
        });
        let queue = Arc::new(QueueCoordinator::new(db.clone(), backend.clone()));
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        let mut executor = ActionExecutor::new(state, db.clone(), bridge, queue);
        if let Some(server) = &server {
            executor = executor.with_server(server.clone());
        }
        Self {
            _temp: temp,
            db,
            log,
            backend,
            server,
            executor,
        }
    }

    fn running(&self) {
        let generation = i64::try_from(self.backend.generation()).unwrap();
        queue::enqueue(
            &self.db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(801),
                app_server_generation: generation,
                prompt: "original A",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        queue::begin_attempt(&self.db, "original", &[], generation).unwrap();
        queue::mark_running(&self.db, "original", "owned-original-turn", generation).unwrap();
    }

    fn no_control_frames(&self) {
        if self.server.is_some() {
            let calls = std::fs::read_to_string(&self.log).unwrap();
            for line in calls.lines() {
                let frame: Value = serde_json::from_str(line).unwrap();
                assert!(
                    !matches!(
                        frame["method"].as_str(),
                        Some("thread/resume" | "thread/fork" | "turn/start" | "turn/interrupt")
                    ),
                    "stop must not infer an owner by RPC: {frame}"
                );
            }
        }
    }
}

async fn exercise(resident: bool, running: bool) {
    let f = Fixture::new(resident).await;
    if running {
        f.running();
    }
    let before = queue::list_filtered(&f.db, Some("thread-b"), None).unwrap();
    let origin = revision::capture(&f.db, Some("thread-b")).unwrap();
    let guard = f.executor.control_lock("thread-b").await.unwrap();
    let stopped = tokio::time::timeout(
        Duration::from_secs(3),
        f.executor.execute(
            CommandAction::Stop {
                reference: Some("thread-b".into()),
            },
            42,
            3,
        ),
    )
    .await;
    drop(guard);
    let after = queue::list_filtered(&f.db, Some("thread-b"), None).unwrap();
    let snapshot = revision::capture(&f.db, Some("thread-b")).unwrap();
    f.no_control_frames();
    let independent = tokio::time::timeout(
        Duration::from_secs(5),
        f.executor.execute_with_context(
            CommandAction::Ask {
                prompt: "independent B".into(),
            },
            ActionContext {
                channel_id: 43,
                user_id: 3,
                discord_message_id: Some(802),
                auto_queue_when_busy: true,
            },
        ),
    )
    .await;
    if let Some(server) = &f.server {
        server.close().await.unwrap();
    }
    let stopped = stopped
        .expect("stop acceptance must not wait on target lock")
        .expect("stop intent must not require an app server or confirmed running turn");
    assert!(stopped.text.contains("Stop accepted"));
    assert!(stopped.text.contains("Execution end is not confirmed"));
    assert!(!stopped.waits_for_final);
    assert_eq!(
        after, before,
        "do not rewrite Starting/Running or discard input"
    );
    assert_eq!(snapshot["stopRevision"], 1);
    assert!(
        revision::validate_in(
            &rusqlite::Connection::open(&f.db).unwrap(),
            Some("thread-b"),
            Some(&origin)
        )
        .is_err()
    );
    assert!(
        control::pending_after(&f.db, 0).unwrap().is_empty(),
        "no fabricated interrupt owner"
    );
    if running {
        assert!(
            cdr_store::execution_hold::reason(&f.db, "original")
                .unwrap()
                .is_some()
        );
    }
    assert!(independent.unwrap().unwrap().waits_for_final);
    assert_eq!(
        *f.backend.starts.lock().await,
        [("thread-a".into(), "independent B".into())]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_original_and_no_resident_still_records_bounded_stop_intent() {
    exercise(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unverified_running_without_resident_is_held_without_queue_rewrite() {
    exercise(false, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_target_with_resident_does_not_wait_for_target_lock_or_query_owner() {
    exercise(true, false).await;
}

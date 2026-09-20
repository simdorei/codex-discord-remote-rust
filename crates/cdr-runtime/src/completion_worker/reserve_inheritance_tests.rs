//! Revision 11: inherit conversations, not permission to replay a failed input.
//! Real queue/controller/worker, persistent offline stdio history and loopback HTTP.
use super::*;
use cdr_app_server::{AppServerConfig, outcomes::parse_thread_turn_states};
use cdr_store::{delivery, observed_completion, queue, reserve_policy};
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::test_support::approval_http as http;

struct Fixture {
    _temp: tempfile::TempDir,
    config: AppServerConfig,
    log: PathBuf,
    db: PathBuf,
    server: Arc<ResidentAppServer>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    transport: http::HttpFixture,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let db = temp.path().join("mirror.sqlite");
        let mut config = crate::soak::native_fixture::config("reserve-auto");
        config.environment.insert(
            "RESERVE_TEST_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        config.environment.insert(
            "RESERVE_TEST_STATE".into(),
            temp.path()
                .join("fixture-history.json")
                .to_string_lossy()
                .into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config.clone()).await.unwrap());
        let queue = Self::coordinator(&db, &server);
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        Self {
            _temp: temp,
            config,
            log,
            db,
            server,
            queue,
            transport: http::start().await,
        }
    }

    fn coordinator(
        db: &std::path::Path,
        server: &Arc<ResidentAppServer>,
    ) -> Arc<QueueCoordinator<AppServerTurnBackend>> {
        Arc::new(QueueCoordinator::new_with_admission_gate(
            db.to_path_buf(),
            Arc::new(AppServerTurnBackend::new(server.clone())),
            crate::restart_readiness::drain::AdmissionGate::new(),
        ))
    }

    async fn rpc(&self, method: &str, mut params: Value) -> Value {
        params["threadId"] = json!("thread-b");
        self.server
            .request(method, params, Duration::from_secs(4), None)
            .await
            .unwrap()
    }

    async fn configure(&self, params: Value) {
        self.rpc("test/configure", params).await;
    }

    async fn submit(&self, id: &str) -> crate::queue_runner::Submission {
        self.queue
            .submit_identified(id, "thread-b", 42, 3, None, id)
            .await
            .unwrap()
    }

    async fn restart(&mut self, new_runtime: bool) {
        if new_runtime {
            self.server.close().await.unwrap();
            self.server = Arc::new(ResidentAppServer::start(self.config.clone()).await.unwrap());
        } else {
            assert!(self.server.force_restart_if_quiescent().await.unwrap());
        }
        self.queue = Self::coordinator(&self.db, &self.server);
    }

    fn worker(&self) -> CompletionWorker {
        CompletionWorker {
            server: self.server.clone(),
            queue: self.queue.clone(),
            http: Arc::new(
                Client::builder()
                    .proxy(self.transport.address.clone(), true)
                    .ratelimiter(None)
                    .build(),
            ),
            commentary_enabled: false,
            history_read_timeout: Duration::from_secs(4),
            commentary: Mutex::new(CommentaryBuffer::default()),
            terminal_fence: terminal_fence::TerminalFence::default(),
        }
    }

    fn frames(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn count(&self, method: &str) -> usize {
        self.frames()
            .iter()
            .filter(|frame| frame["method"] == method)
            .count()
    }

    async fn assert_failed_history(&self, turn: &str) {
        let history = self.rpc("thread/read", json!({"includeTurns":true})).await;
        let states = parse_thread_turn_states(&history, "thread-b").unwrap();
        let failed = states
            .get(turn)
            .expect("restarted child must retain the exact old turn");
        assert_eq!(failed.status, TurnStatus::Failed);
        assert!(failed.usage_limit);
    }

    async fn close(self) -> Vec<Value> {
        self.server.close().await.unwrap();
        self.transport.stop.send(()).unwrap();
        self.transport
            .task
            .await
            .unwrap()
            .into_iter()
            .map(|(_, body)| body)
            .collect()
    }
}

fn failed_count(messages: &[Value]) -> usize {
    messages
        .iter()
        .filter(|message| {
            message["content"]
                .as_str()
                .is_some_and(|s| s.starts_with("Failed"))
        })
        .count()
}

#[tokio::test]
async fn inherited_terminal_revalidates_and_starts_only_the_distinct_queued_input() {
    let mut f = Fixture::new().await;
    f.configure(json!({"ordinary":true})).await;
    let first = f.submit("failed-original").await;
    let turn = first.turn_id.unwrap();
    assert!(f.submit("distinct-next").await.queued);
    f.rpc("test/complete", json!({"turnId":turn,"status":"failed"}))
        .await;
    assert!(reserve_policy::get(&f.db, "thread-b").unwrap().is_none());
    let execution = queue::list(&f.db)
        .unwrap()
        .into_iter()
        .find(|j| j.job_id == "failed-original")
        .unwrap();
    f.restart(false).await;
    assert_ne!(
        execution.execution_generation,
        Some(i64::try_from(f.server.generation()).unwrap())
    );
    f.assert_failed_history(&turn).await;
    // No old completion is passed directly into the coordinator or worker.
    let worker = f.worker();
    worker.recover().await.unwrap();
    worker.recover().await.unwrap();
    assert_eq!(f.count("thread/settings/update"), 0);
    assert_eq!(f.count("turn/start"), 2);
    assert!(!reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap());
    let jobs = queue::list(&f.db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "distinct-next");
    assert_eq!(jobs[0].state, queue::QueueJobState::Running);
    assert_eq!(
        jobs[0].execution_generation,
        Some(i64::try_from(f.server.generation()).unwrap())
    );
    assert!(delivery::list_pending(&f.db).unwrap().is_empty());
    assert!(!observed_completion::contains(&f.db, "thread-b", &turn).unwrap());
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|v| v["event"] == "start_settings")
        .collect();
    assert_eq!(starts[0]["settings"]["model"], "model-a");
    assert_eq!(starts[1]["settings"]["model"], "model-a");
    assert_eq!(failed_count(&f.close().await), 1);
}

#[tokio::test]
async fn inherited_terminal_with_reused_generation_or_legacy_null_is_a_recheck_not_a_replay() {
    for (new_runtime, legacy_null) in [(false, false), (true, false), (true, true), (false, true)] {
        let mut f = Fixture::new().await;
        f.configure(json!({"ordinary":true})).await;
        let turn = f.submit("inherited-original").await.turn_id.unwrap();
        f.rpc("test/complete", json!({"turnId":turn,"status":"failed"}))
            .await;
        let old_generation = f.server.generation();
        if legacy_null {
            rusqlite::Connection::open(&f.db).unwrap().execute(
                "UPDATE codex_turn_queue SET execution_generation=NULL WHERE job_id='inherited-original'", [],
            ).unwrap();
        }
        f.restart(new_runtime).await;
        assert_eq!(old_generation == f.server.generation(), new_runtime);
        f.assert_failed_history(&turn).await;
        let worker = f.worker();
        worker.recover().await.unwrap();
        worker.recover().await.unwrap();
        assert_eq!(
            f.count("thread/settings/update"),
            0,
            "new_runtime={new_runtime} legacy_null={legacy_null}"
        );
        assert_eq!(f.count("turn/start"), 1);
        assert!(queue::list(&f.db).unwrap().is_empty());
        assert!(!reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap());
        assert_eq!(failed_count(&f.close().await), 1);
    }
}

#[tokio::test]
async fn inherited_terminal_keeps_manual_off_and_current_quota_unknown_safe() {
    for mode in ["manual", "off", "unknown-quota", "unknown-settings"] {
        let mut f = Fixture::new().await;
        f.configure(json!({"ordinary":true})).await;
        let turn = f.submit("held-original").await.turn_id.unwrap();
        f.rpc("test/complete", json!({"turnId":turn,"status":"failed"}))
            .await;
        if matches!(mode, "manual" | "off") {
            reserve_policy::set_mode(&f.db, "thread-b", mode).unwrap();
        } else if mode == "unknown-settings" {
            reserve_policy::mark_unknown(&f.db, "thread-b", "unconfirmed prior settings").unwrap();
        }
        f.restart(false).await;
        if mode == "unknown-quota" {
            f.configure(json!({"ordinary":null})).await;
        }
        f.assert_failed_history(&turn).await;
        let worker = f.worker();
        worker.recover().await.unwrap();
        worker.recover().await.unwrap();
        assert_eq!(f.count("thread/settings/update"), 0, "{mode}");
        assert_eq!(f.count("turn/start"), 1, "{mode}");
        assert!(queue::list(&f.db).unwrap().is_empty());
        assert!(!reserve_policy::usage_failure_unresolved(&f.db, "thread-b").unwrap());
        assert_eq!(failed_count(&f.close().await), 1);
    }
}

#[path = "goal_inheritance_tests.rs"]
mod goal_inheritance_tests;

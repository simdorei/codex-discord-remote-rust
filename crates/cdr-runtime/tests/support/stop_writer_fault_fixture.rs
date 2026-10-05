use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use cdr_app_server::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ResidentAppServer,
};
use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::{
    ingress::stop::{StopScope, accept_unresolved},
    queue::{self, QueueJobState, StoredQueueJob},
};
use rusqlite::{Connection, types::ValueRef};
use serde_json::{Value, json};

pub const TARGET: &str = "thread-b";
pub const OTHER: &str = "independent-b";
pub const INPUT: &str = "original writer fault input, never replay";

struct Fence {
    inner: RuntimeDeadGenerationFence,
    root: PathBuf,
    db: PathBuf,
    crash: bool,
    fired: AtomicBool,
}

impl Fence {
    fn crash_after_commit(&self, owner: (&str, u64), request: (&str, &RequestId)) {
        let receipt = accept_unresolved(
            &self.db,
            StopScope {
                target: TARGET,
                channel: 99,
                owner: 20,
            },
            &json!({"target":TARGET,"route":"Explicit","command":{"Stop":{"reference":TARGET}}}),
            None,
            || Ok(()),
        )
        .unwrap()
        .unwrap();
        let snapshot = Snapshot::read(&self.db);
        assert_eq!(
            receipt.jobs.as_slice(),
            std::slice::from_ref(&snapshot.queue.job_id)
        );
        snapshot.assert_prepared();
        assert!(!snapshot.holds.is_empty());
        assert_eq!(snapshot.receipts.len(), 1);
        std::fs::write(
            self.root.join("ready.tmp"),
            serde_json::to_vec(&json!({
                "stage":"writer-and-stop-committed-before-first-byte",
                "owner":owner.0,"generation":owner.1,"attempt":request.0,"wire":request.1,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::rename(self.root.join("ready.tmp"), self.root.join("ready.json")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.root.join("exit-now").is_file() {
            assert!(
                Instant::now() < deadline,
                "parent never confirmed durable crash boundary"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // No unwinding, Drop, finish_mutation, close, or graceful runtime shutdown.
        std::process::exit(73);
    }
}

impl DeadGenerationFence for Fence {
    fn persist(&self, work: &DeadGenerationWork) -> Result<(), AppServerError> {
        self.inner.persist(work)
    }
    fn check_request(
        &self,
        generation: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), AppServerError> {
        self.inner.check_request(generation, method, params)
    }
    fn request_origin(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Option<Value>, AppServerError> {
        self.inner.request_origin(method, params)
    }
    fn begin_mutation_with_origin(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        scoped: bool,
        origin: Option<&Value>,
    ) -> Result<bool, AppServerError> {
        self.inner
            .begin_mutation_with_origin(owner, request, method, params, scoped, origin)
    }
    fn begin_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        scoped: bool,
    ) -> Result<bool, AppServerError> {
        self.inner
            .begin_mutation(owner, request, method, params, scoped)
    }
    fn begin_queue_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        claim: &Value,
    ) -> Result<bool, AppServerError> {
        let committed = self
            .inner
            .begin_queue_mutation(owner, request, method, params, claim)?;
        if self.crash
            && method == "turn/start"
            && params["threadId"] == TARGET
            && !self.fired.swap(true, Ordering::AcqRel)
        {
            assert!(committed);
            self.crash_after_commit(owner, request);
        }
        Ok(committed)
    }
    fn finish_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        outcome: &str,
    ) -> Result<(), AppServerError> {
        self.inner.finish_mutation(owner, request, outcome)
    }
}

pub struct Fixture {
    pub db: PathBuf,
    pub log: PathBuf,
    pub server: Arc<ResidentAppServer>,
    pub queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    executor: ActionExecutor<AppServerTurnBackend>,
}

impl Fixture {
    pub async fn start(root: &Path, runtime: &str, crash: bool) -> Self {
        let db = root.join("mirror.sqlite");
        let state = root.join("state.sqlite");
        let log = root.join(format!("{runtime}-rpc.jsonl"));
        if !state.exists() {
            Connection::open(&state)
                .unwrap()
                .execute_batch(include_str!("../fixtures/action_state.sql"))
                .unwrap();
        }
        let inner = RuntimeDeadGenerationFence::new(db.clone(), runtime.into(), None).unwrap();
        let fence = Arc::new(Fence {
            inner,
            root: root.into(),
            db: db.clone(),
            crash,
            fired: AtomicBool::new(false),
        });
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, fence)
                .await
                .unwrap(),
        );
        let queue = Arc::new(QueueCoordinator::new(
            db.clone(),
            Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
        ));
        let executor = ActionExecutor::new(
            state,
            db.clone(),
            Arc::new(BridgeState::new(root.join("bridge.json"))),
            Arc::clone(&queue),
        )
        .with_server(Arc::clone(&server));
        Self {
            db,
            log,
            server,
            queue,
            executor,
        }
    }

    pub async fn stop(&self) {
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            self.executor.execute(
                CommandAction::Stop {
                    reference: Some(TARGET.into()),
                },
                99,
                20,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.text.contains("Stop accepted"));
        assert!(response.text.contains("Execution end is not confirmed"));
        assert!(!response.waits_for_final);
    }

    pub async fn prove_cold_no_replay(&self, before: &Snapshot) {
        for _ in 0..2 {
            tokio::time::timeout(Duration::from_secs(5), self.queue.kick_target(TARGET))
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), self.queue.recover())
                .await
                .unwrap()
                .unwrap();
        }
        let attempted = self
            .server
            .execute_queue_turn(
                cdr_app_server::requests::start_turn(TARGET, INPUT),
                self.server.generation(),
                serde_json::to_value(&before.queue).unwrap(),
            )
            .await;
        assert!(
            attempted.is_err(),
            "old claim cannot borrow a new wire occurrence"
        );
        before.assert_retained(&Snapshot::read(&self.db));
        // Budget starts on an already healthy replacement, not the damaged pipe.
        let b = tokio::time::timeout(
            Duration::from_secs(5),
            self.queue
                .submit(OTHER, 100, 21, Some(902), "independent healthy B"),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(b.turn_id.is_some());
        assert_eq!(calls(&self.log, "turn/start", OTHER), 1);
        assert_eq!(calls(&self.log, "turn/start", TARGET), 0);
        assert_eq!(calls(&self.log, "turn/interrupt", TARGET), 0);
        before.assert_retained(&Snapshot::read(&self.db));
    }
}

#[derive(Debug, PartialEq)]
pub struct Snapshot {
    pub queue: StoredQueueJob,
    pub attempts: Vec<Value>,
    pub holds: Vec<Value>,
    pub receipts: Vec<Value>,
    pub revisions: Vec<Value>,
    pub controls: Vec<Value>,
}

impl Snapshot {
    pub fn read(path: &Path) -> Self {
        let db = Connection::open(path).unwrap();
        Self {
            queue: queue::list_filtered(path, Some(TARGET), None)
                .unwrap()
                .remove(0),
            attempts: rows(
                &db,
                "SELECT * FROM codex_mutation_attempts WHERE target_thread_id=? AND method='turn/start' ORDER BY rowid",
            ),
            holds: rows(
                &db,
                "SELECT * FROM cdr_execution_holds WHERE target_thread_id=? ORDER BY rowid",
            ),
            receipts: rows(
                &db,
                "SELECT * FROM cdr_stop_revision_receipts WHERE target_thread_id=? ORDER BY rowid",
            ),
            revisions: rows(
                &db,
                "SELECT * FROM cdr_stop_revisions WHERE target_thread_id=? ORDER BY rowid",
            ),
            controls: rows(
                &db,
                "SELECT * FROM cdr_stop_controls WHERE target_thread_id=? ORDER BY rowid",
            ),
        }
    }

    pub fn assert_prepared(&self) {
        assert_eq!(self.queue.state, QueueJobState::Starting);
        assert_eq!(self.queue.prompt, INPUT);
        assert_eq!(self.queue.attempt_count, 1);
        assert_eq!(self.attempts.len(), 1);
        assert!(
            self.attempts[0]
                .as_object()
                .unwrap()
                .values()
                .any(|value| value.as_str() == Some("prepared")),
            "{:?}",
            self.attempts
        );
    }

    pub fn assert_retained(&self, after: &Self) {
        let a = &self.queue;
        let b = &after.queue;
        assert_eq!(b.state, QueueJobState::Starting);
        assert_eq!(
            (
                &a.job_id,
                &a.target_thread_id,
                a.channel_id,
                a.owner_user_id,
                a.discord_message_id
            ),
            (
                &b.job_id,
                &b.target_thread_id,
                b.channel_id,
                b.owner_user_id,
                b.discord_message_id
            )
        );
        assert_eq!(
            (&a.prompt, a.attempt_count, &a.turn_id, &a.baseline_turn_ids),
            (&b.prompt, b.attempt_count, &b.turn_id, &b.baseline_turn_ids)
        );
        assert_eq!(
            (
                a.app_server_generation,
                a.execution_generation,
                a.turn_observation_generation
            ),
            (
                b.app_server_generation,
                b.execution_generation,
                b.turn_observation_generation
            )
        );
        assert_eq!(a.created_at.to_bits(), b.created_at.to_bits());
        assert_eq!(self.attempts.len(), after.attempts.len());
        assert_eq!(after.attempts[0]["state"], "prepared");
        for (before, after) in self.attempts.iter().zip(&after.attempts) {
            for (key, value) in before.as_object().unwrap() {
                // Failure classification/timestamps may legitimately be recorded.
                if !matches!(
                    key.as_str(),
                    "state" | "phase" | "outcome" | "updated_at" | "completed_at" | "error"
                ) {
                    assert_eq!(
                        after.get(key),
                        Some(value),
                        "immutable mutation field {key}"
                    );
                }
            }
        }
        assert_eq!(self.holds, after.holds);
        assert_eq!(self.receipts, after.receipts);
        assert_eq!(self.revisions, after.revisions);
        assert!(
            after.controls.is_empty(),
            "no invented interrupt or execution-end control"
        );
    }
}

fn rows(db: &Connection, sql: &str) -> Vec<Value> {
    let mut statement = db.prepare(sql).unwrap();
    let columns: Vec<String> = statement
        .column_names()
        .iter()
        .map(|s| (*s).into())
        .collect();
    statement
        .query_map([TARGET], |row| {
            let mut value = serde_json::Map::new();
            for (index, name) in columns.iter().enumerate() {
                let item = match row.get_ref(index)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(value) => json!(value),
                    ValueRef::Real(value) => json!(format!("{value:.17e}")),
                    ValueRef::Text(value) => json!(String::from_utf8_lossy(value)),
                    ValueRef::Blob(value) => json!(hex::encode(value)),
                };
                value.insert(name.clone(), item);
            }
            Ok(Value::Object(value))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

pub fn calls(path: &Path, method: &str, target: &str) -> usize {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|row| row["method"] == method && row["params"]["threadId"] == target)
        .count()
}

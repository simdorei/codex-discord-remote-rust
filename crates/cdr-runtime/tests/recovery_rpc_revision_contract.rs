use cdr_app_server::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ResidentAppServer,
    requests::AppRequest,
};
use cdr_runtime::{
    app_backend::AppServerTurnBackend, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::queue::{self, NewQueueJob};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[path = "support/action_app_server.rs"]
mod server_support;

#[derive(Clone, Copy)]
enum Timing {
    Before,
    After,
}

struct Fence {
    inner: RuntimeDeadGenerationFence,
    db: PathBuf,
    timing: Timing,
    fired: AtomicBool,
    original_origin: Mutex<Option<Value>>,
}
impl Fence {
    fn cancel(&self) {
        let cancelled = queue::cancel_for_recovery(&self.db, "thread-b", 42, 3, 2.0).unwrap();
        assert_eq!(cancelled.jobs, ["original"]);
    }
    fn begin(&self, method: &str, params: &Value) -> bool {
        let intercept = method == "thread/settings/update"
            && params["threadId"] == "thread-b"
            && !self.fired.swap(true, Ordering::AcqRel);
        if intercept && matches!(self.timing, Timing::Before) {
            self.cancel();
        }
        intercept
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
        let origin = self.inner.request_origin(method, params)?;
        if method == "thread/settings/update" && params["threadId"] == "thread-b" {
            self.original_origin.lock().unwrap().clone_from(&origin);
        }
        Ok(origin)
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
        let intercept = self.begin(method, params);
        let result = self
            .inner
            .begin_mutation_with_origin(owner, request, method, params, scoped, origin)?;
        if intercept && matches!(self.timing, Timing::After) {
            self.cancel();
        }
        Ok(result)
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
        self.inner
            .begin_queue_mutation(owner, request, method, params, claim)
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

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: Arc<ResidentAppServer>,
    fence: Arc<Fence>,
}
impl Fixture {
    async fn new(timing: Timing) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("store.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let inner =
            RuntimeDeadGenerationFence::new(db.clone(), "recovery-revision".into(), None).unwrap();
        let fence = Arc::new(Fence {
            inner,
            db: db.clone(),
            timing,
            fired: AtomicBool::new(false),
            original_origin: Mutex::new(None),
        });
        let mut config = native_fixture::config("action");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(
            ResidentAppServer::start_with_dead_generation_fence(config, fence.clone())
                .await
                .unwrap(),
        );
        queue::enqueue(
            &db,
            NewQueueJob {
                job_id: "original",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(801),
                app_server_generation: i64::try_from(server.generation()).unwrap(),
                prompt: "original A",
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
            fence,
        }
    }
}

async fn exercise(timing: Timing) {
    let f = Fixture::new(timing).await;
    let result = f
        .server
        .execute(
            AppRequest {
                method: "thread/settings/update",
                params: json!({"threadId":"thread-b","model":"gpt-5.4",
            "reasoningEffort":"high","serviceTier":"default"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await;
    let queue = QueueCoordinator::new(
        f.db.clone(),
        Arc::new(AppServerTurnBackend::new(f.server.clone())),
    );
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        queue.submit("thread-c", 43, 4, Some(802), "independent B"),
    )
    .await
    .unwrap()
    .unwrap();
    let calls = server_support::rpc_log(&f.log);
    let count = |method: &str, target: &str| {
        calls
            .iter()
            .filter(|r| r["method"] == method && r["params"]["threadId"] == target)
            .count()
    };
    let quarantined = f.server.lifecycle_snapshot().await.quarantined;
    f.server.close().await.unwrap();
    assert!(f.fence.fired.load(Ordering::Acquire));
    let origin = f.fence.original_origin.lock().unwrap().clone().unwrap();
    assert_eq!(
        origin,
        json!({"target":"thread-b","stopRevision":0}),
        "use the actual admission origin captured before the cancellation"
    );
    assert!(b.turn_id.is_some());
    assert_eq!(count("turn/start", "thread-c"), 1);
    assert_eq!(
        count("turn/start", "thread-b"),
        0,
        "cancelled A is never sent"
    );
    assert_eq!(
        count("turn/interrupt", "thread-b"),
        0,
        "cancellation is not an interrupt"
    );
    assert!(
        queue::list_filtered(&f.db, Some("thread-b"), None)
            .unwrap()
            .is_empty()
    );
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original")
            .unwrap()
            .is_some()
    );
    assert!(!quarantined);
    match timing {
        Timing::Before => {
            assert!(
                result.is_err(),
                "old ordinary RPC admitted after recovery cancellation"
            );
            assert_eq!(count("thread/settings/update", "thread-b"), 0);
        }
        Timing::After => {
            assert!(result.is_ok());
            assert_eq!(
                count("thread/settings/update", "thread-b"),
                1,
                "writer-first is one consumed occurrence, never permission to replay"
            );
        }
    }
}

#[tokio::test]
async fn recovery_before_writer_blocks_the_original_rpc_and_native_b_progresses() {
    exercise(Timing::Before).await;
}

#[tokio::test]
async fn writer_before_recovery_remains_one_consumed_native_occurrence() {
    exercise(Timing::After).await;
}

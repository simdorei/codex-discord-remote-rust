use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use cdr_app_server::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ResidentAppServer,
    requests::AppRequest,
};
use cdr_runtime::{
    app_backend::AppServerTurnBackend, dead_generation_recovery::RuntimeDeadGenerationFence,
    queue_runner::QueueCoordinator, soak::native_fixture,
};
use cdr_store::{
    ingress::stop::{StopScope, accept_nonrunning},
    queue::{self, NewQueueJob},
};
use serde_json::{Value, json};

#[path = "stop_rpc_revision_contract/managed.rs"]
mod managed;
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
}

impl Fence {
    fn stop(&self) {
        let receipt = accept_nonrunning(&self.db,
            StopScope {target:"thread-b", channel:42, owner:3},
            &json!({"target":"thread-b","route":"Explicit","command":{"Stop":{"reference":"thread-b"}}}),
            None, || Ok(()),
        ).unwrap().expect("pending original must accept stop");
        assert_eq!(receipt.jobs, ["original"]);
    }

    fn begin(&self, method: &str, params: &Value) -> bool {
        let intercept = method == "thread/settings/update"
            && params["threadId"] == "thread-b"
            && !self.fired.swap(true, Ordering::AcqRel);
        if intercept && matches!(self.timing, Timing::Before) {
            self.stop();
        }
        intercept
    }

    fn after(&self, intercept: bool) {
        if intercept && matches!(self.timing, Timing::After) {
            self.stop();
        }
    }
}

impl DeadGenerationFence for Fence {
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
        let intercept = self.begin(method, params);
        if intercept {
            assert_eq!(origin, Some(&json!({"target":"thread-b","stopRevision":0})));
        }
        let result = self
            .inner
            .begin_mutation_with_origin(owner, request, method, params, scoped, origin)?;
        self.after(intercept);
        Ok(result)
    }
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
    fn begin_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        scoped: bool,
    ) -> Result<bool, AppServerError> {
        let intercept = self.begin(method, params);
        let result = self
            .inner
            .begin_mutation(owner, request, method, params, scoped)?;
        self.after(intercept);
        Ok(result)
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

fn original_job(generation: u64) -> NewQueueJob<'static> {
    NewQueueJob {
        job_id: "original",
        target_thread_id: "thread-b",
        channel_id: 42,
        owner_user_id: Some(3),
        discord_message_id: Some(801),
        app_server_generation: i64::try_from(generation).unwrap(),
        prompt: "original A",
        queued: true,
        ack_sent: true,
        created_at: 1.0,
    }
}

async fn exercise(timing: Timing) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let inner = RuntimeDeadGenerationFence::new(db.clone(), "revision-race".into(), None).unwrap();
    let fence = Arc::new(Fence {
        inner,
        db: db.clone(),
        timing,
        fired: AtomicBool::new(false),
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
    let original = queue::enqueue(&db, original_job(server.generation()))
        .unwrap()
        .job;
    let result = server
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
    let coordinator = QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(Arc::clone(&server))),
    );
    let independent = tokio::time::timeout(
        Duration::from_secs(5),
        coordinator.submit("thread-c", 43, 4, Some(802), "independent B"),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(independent.turn_id.is_some());
    server
        .execute(
            AppRequest {
                method: "thread/read",
                params: json!({"threadId":"thread-c"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let calls = server_support::rpc_log(&log);
    let updates = calls
        .iter()
        .filter(|r| r["method"] == "thread/settings/update")
        .count();
    let starts = calls
        .iter()
        .filter(|r| r["method"] == "turn/start" && r["params"]["threadId"] == "thread-c")
        .count();
    let original_after = queue::list_filtered(&db, Some("thread-b"), None).unwrap();
    let quarantined = server.lifecycle_snapshot().await.quarantined;
    server.close().await.unwrap();
    assert!(fence.fired.load(Ordering::Acquire));
    assert_eq!(starts, 1);
    assert_eq!(original_after, [original]);
    assert!(
        cdr_store::execution_hold::reason(&db, "original")
            .unwrap()
            .is_some()
    );
    assert!(!quarantined);
    match timing {
        Timing::Before => {
            assert!(
                result.is_err(),
                "an RPC waiting before writer admission cannot borrow the post-stop revision"
            );
            assert_eq!(updates, 0);
        }
        Timing::After => {
            assert!(result.is_ok());
            assert_eq!(
                updates, 1,
                "writer-first remains one consumed occurrence, not replay permission"
            );
        }
    }
}

#[tokio::test]
async fn generic_settings_stop_before_writer_blocks_the_original_occurrence() {
    exercise(Timing::Before).await;
}
#[tokio::test]
async fn generic_settings_writer_before_stop_retains_exactly_one_send() {
    exercise(Timing::After).await;
}

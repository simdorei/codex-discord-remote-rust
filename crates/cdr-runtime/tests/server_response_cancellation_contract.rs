use cdr_app_server::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, RequestId, ResidentAppServer,
    RpcErrorPayload, ServerRequest,
};
use cdr_runtime::{dead_generation_recovery::RuntimeDeadGenerationFence, soak::native_fixture};
use cdr_store::queue;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[path = "support/response_stop_fence.rs"]
mod stop_fence;

struct Fixture {
    _temp: tempfile::TempDir,
    db: PathBuf,
    log: PathBuf,
    server: ResidentAppServer,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_writer_stop(false).await
    }

    async fn with_writer_stop(late: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let fence =
            RuntimeDeadGenerationFence::new(db.clone(), "response-test".into(), None).unwrap();
        let fence: Arc<dyn DeadGenerationFence> = if late {
            Arc::new(stop_fence::StopAtWriter::new(fence, db.clone()))
        } else {
            Arc::new(fence)
        };
        let mut config = native_fixture::config("approval");
        config
            .environment
            .insert("CDR_APPROVAL_TEST_LOG".into(), log.to_string_lossy().into());
        let server = ResidentAppServer::start_with_dead_generation_fence(config, fence)
            .await
            .unwrap();
        for (target, turn, job, channel) in [
            ("thread-b", "turn-b", "original-a", 42),
            ("independent-b", "independent-turn", "original-b", 43),
        ] {
            cdr_store::mapping::upsert_thread(&db, target, "project", "fixture", 100, channel, 1.0)
                .unwrap();
            queue::enqueue(
                &db,
                queue::NewQueueJob {
                    job_id: job,
                    target_thread_id: target,
                    channel_id: channel,
                    owner_user_id: Some(3),
                    discord_message_id: Some(channel + 100),
                    app_server_generation: i64::try_from(server.generation()).unwrap(),
                    prompt: "original fixture request",
                    queued: false,
                    ack_sent: true,
                    created_at: 1.0,
                },
            )
            .unwrap();
            cdr_store::schema::open_initialized(&db).unwrap().execute(
                "UPDATE codex_turn_queue SET state='running',turn_id=?,attempt_count=1 WHERE job_id=?",
                rusqlite::params![turn, job],
            ).unwrap();
        }
        Self {
            _temp: temp,
            db,
            log,
            server,
        }
    }

    async fn pending(&self, target: &str, turn: &str, id: &str) -> ServerRequest {
        self.server
            .request(
                "test/pending",
                json!({
                    "threadId": target, "turnId": turn, "requestId": id,
                }),
                Duration::from_secs(2),
                Some(self.server.generation()),
            )
            .await
            .unwrap();
        self.server
            .pending_server_requests(Some(target))
            .await
            .unwrap()
            .into_iter()
            .find(|request| request.id == cdr_app_server::RequestId::String(id.into()))
            .unwrap()
    }

    fn hold_original(&self) {
        // The exact durable per-original hold produced by stop. Never a target-wide mock.
        cdr_store::schema::open_initialized(&self.db).unwrap().execute(
            "INSERT INTO cdr_execution_holds(job_id,target_thread_id,reason,evidence_json,created_at)
             VALUES('original-a','thread-b','original stop accepted','{}',1)", [],
        ).unwrap();
    }

    async fn independent_progress(&self) {
        let b = self
            .pending("independent-b", "independent-turn", "reply-b")
            .await;
        self.server
            .respond_current(
                &b.id,
                b.occurrence,
                json!({"decision":"accept"}),
                self.server.generation(),
            )
            .await
            .unwrap();
        // A real protocol round trip is the log-consumption barrier, not a sleep.
        self.server
            .request(
                "test/finish",
                json!({}),
                Duration::from_secs(2),
                Some(self.server.generation()),
            )
            .await
            .unwrap();
    }

    async fn finish(self, blocked: bool) {
        let healthy = !self.server.lifecycle_snapshot().await.quarantined;
        self.server.close().await.unwrap();
        let values = std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let replies = |id: &str| {
            values
                .iter()
                .filter(|value| {
                    value["id"] == id
                        && (value.get("result").is_some() || value.get("error").is_some())
                })
                .count()
        };
        assert_eq!(
            replies("reply-a"),
            0,
            "held original response must not reach the native writer"
        );
        assert!(
            blocked,
            "a permanent original hold must refuse the response"
        );
        assert_eq!(
            replies("reply-b"),
            1,
            "independent B must progress on the same resident"
        );
        assert!(
            healthy,
            "a local original hold is not a shared transport failure"
        );
    }
}

#[tokio::test]
async fn stopped_original_ordinary_response_is_not_written() {
    let f = Fixture::new().await;
    let a = f.pending("thread-b", "turn-b", "reply-a").await;
    f.hold_original();
    let blocked = f
        .server
        .respond(
            &a.id,
            a.occurrence,
            json!({"decision":"accept"}),
            f.server.generation(),
        )
        .await
        .is_err();
    f.independent_progress().await;
    f.finish(blocked).await;
}

#[tokio::test]
async fn stopped_original_current_response_is_not_written() {
    let f = Fixture::new().await;
    let a = f.pending("thread-b", "turn-b", "reply-a").await;
    f.hold_original();
    let blocked = f
        .server
        .respond_current(
            &a.id,
            a.occurrence,
            json!({"decision":"accept"}),
            f.server.generation(),
        )
        .await
        .is_err();
    f.independent_progress().await;
    f.finish(blocked).await;
}

#[tokio::test]
async fn stopped_original_error_response_is_not_written() {
    let f = Fixture::new().await;
    let a = f.pending("thread-b", "turn-b", "reply-a").await;
    f.hold_original();
    let blocked = f
        .server
        .respond_error(
            &a.id,
            a.occurrence,
            RpcErrorPayload {
                code: -32_800,
                message: "fixture cancellation".into(),
                data: None,
            },
            f.server.generation(),
        )
        .await
        .is_err();
    f.independent_progress().await;
    f.finish(blocked).await;
}

#[tokio::test]
async fn recovery_cancelled_original_response_is_not_written() {
    let f = Fixture::new().await;
    let a = f.pending("thread-b", "turn-b", "reply-a").await;
    let cancelled = queue::cancel_for_recovery(&f.db, "thread-b", 42, 3, 2.0).unwrap();
    assert_eq!(cancelled.jobs, ["original-a"]);
    let blocked = f
        .server
        .respond_current(
            &a.id,
            a.occurrence,
            json!({"decision":"accept"}),
            f.server.generation(),
        )
        .await
        .is_err();
    f.independent_progress().await;
    f.finish(blocked).await;
}

#[tokio::test]
async fn stop_committed_at_final_writer_rejects_the_frozen_original() {
    let f = Fixture::with_writer_stop(true).await;
    let a = f.pending("thread-b", "turn-b", "reply-a").await;
    let blocked = f
        .server
        .respond_current(
            &a.id,
            a.occurrence,
            json!({"decision":"accept"}),
            f.server.generation(),
        )
        .await
        .is_err();
    assert!(
        cdr_store::execution_hold::reason(&f.db, "original-a")
            .unwrap()
            .is_some()
    );
    f.independent_progress().await;
    f.finish(blocked).await;
}

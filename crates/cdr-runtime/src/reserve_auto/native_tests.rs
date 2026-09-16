//! Real Reserve controller/backend/queue over the native offline stdio fixture.
use crate::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
    reserve_auto::ReserveAutoController,
};
use cdr_app_server::{ResidentAppServer, outcomes::parse_turn_completion};
use cdr_store::reserve_policy;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Fixture {
    temp: tempfile::TempDir,
    log: PathBuf,
    db: PathBuf,
    server: Arc<ResidentAppServer>,
    controller: Arc<ReserveAutoController>,
    queue: Arc<QueueCoordinator<AppServerTurnBackend>>,
    executor: Arc<ActionExecutor<AppServerTurnBackend>>,
    gate: crate::restart_readiness::drain::AdmissionGate,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let db = temp.path().join("mirror.sqlite");
        let mut config = crate::soak::native_fixture::config("reserve-auto");
        config
            .environment
            .insert("RESERVE_TEST_LOG".into(), log.to_string_lossy().into());
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let controller = ReserveAutoController::new(server.clone(), db.clone());
        let gate = crate::restart_readiness::drain::AdmissionGate::new();
        let queue = Arc::new(QueueCoordinator::new_with_admission_gate(
            db.clone(),
            Arc::new(
                AppServerTurnBackend::new(server.clone()).with_reserve_auto(controller.clone()),
            ),
            gate.clone(),
        ));
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("../../tests/fixtures/action_state.sql"))
            .unwrap();
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        let executor = Arc::new(
            ActionExecutor::new(state, db.clone(), bridge, queue.clone())
                .with_server(server.clone())
                .with_reserve_auto(controller.clone()),
        );
        Self {
            temp,
            log,
            db,
            server,
            controller,
            queue,
            executor,
            gate,
        }
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
            .filter(|row| row["method"] == method)
            .count()
    }
    async fn submit(&self, id: &str) -> crate::queue_runner::Submission {
        self.queue
            .submit_identified(id, "thread-b", 42, 3, None, id)
            .await
            .unwrap()
    }
    async fn finish(&self, turn: &str, failed: bool) {
        let response = self
            .rpc(
                "test/complete",
                json!({"turnId":turn,"status":if failed {"failed"} else {"completed"}}),
            )
            .await;
        let completion = parse_turn_completion(&response, false).unwrap();
        assert_eq!(completion.usage_limit, failed);
        self.queue
            .stage_turn_completion_with_usage_limit(
                "thread-b",
                turn,
                "fixture final",
                completion.usage_limit,
            )
            .await
            .unwrap();
    }
    async fn close(self) {
        self.server.close().await.unwrap();
    }
}

#[tokio::test]
async fn native_auto_reserve_runs_two_new_requests_then_restores_exact_settings() {
    let f = Fixture::new().await;
    let a = f.submit("job-a").await;
    f.finish(a.turn_id.as_deref().unwrap(), false).await;
    let b = f.submit("job-b").await;
    f.finish(b.turn_id.as_deref().unwrap(), false).await;
    f.configure(json!({"ordinary":true})).await;
    let c = f.submit("job-c").await;
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|r| r["event"] == "start_settings")
        .collect();
    assert_eq!(starts.len(), 3);
    assert_eq!(starts[0]["settings"]["model"], "gpt-reserve");
    assert_eq!(starts[1]["settings"]["model"], "gpt-reserve");
    assert_eq!(
        starts[2]["settings"],
        json!({"model":"model-a","effort":"high","serviceTier":"priority"})
    );
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .state,
        "ordinary"
    );
    f.finish(c.turn_id.as_deref().unwrap(), false).await;
    f.close().await;
}

#[tokio::test]
async fn native_start_usage_is_never_replayed_and_next_new_job_runs() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true,"reject_next_start":"usage"}))
        .await;
    let a = f.submit("rejected-a").await;
    assert!(a.turn_id.is_none());
    assert_eq!(
        a.warning.unwrap().kind,
        crate::queue_runner::BackendFailureKind::AutoReserveHeld
    );
    let b = f.submit("new-b").await;
    f.finish(b.turn_id.as_deref().unwrap(), false).await;
    f.queue.kick_target("thread-b").await.unwrap();
    assert_eq!(f.count("turn/start"), 2);
    assert_eq!(
        cdr_store::queue::list(&f.db)
            .unwrap()
            .iter()
            .filter(|j| j.job_id == "rejected-a")
            .count(),
        1
    );
    f.close().await;
}

#[tokio::test]
async fn native_terminal_usage_switches_before_next_queue_job_and_duplicate_does_not_replay() {
    let f = Fixture::new().await;
    f.configure(json!({"ordinary":true})).await;
    let a = f.submit("terminal-a").await;
    let b = f.submit("next-b").await;
    assert!(b.queued);
    let turn = a.turn_id.as_deref().unwrap();
    f.finish(turn, true).await;
    assert!(
        f.queue
            .stage_turn_completion_with_usage_limit("thread-b", turn, "duplicate", true)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(f.count("turn/start"), 2);
    let starts: Vec<_> = f
        .frames()
        .into_iter()
        .filter(|r| r["event"] == "start_settings")
        .collect();
    assert_eq!(starts[1]["settings"]["model"], "gpt-reserve");
    f.close().await;
}

#[tokio::test]
async fn native_account_change_cannot_reuse_a_previous_automatic_episode() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"account":"account-b"})).await;
    assert!(
        f.controller.prepare_turn("thread-b").await.is_err(),
        "foreign account was allowed to reuse the old automatic episode"
    );
    assert_eq!(f.count("turn/start"), 0);
    f.close().await;
}

#[tokio::test]
async fn native_changed_or_unsupported_effort_does_not_pass_by_falling_back_in_validation() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"settings":{"model":"gpt-reserve","effort":"invalid-effort","serviceTier":"default"}})).await;
    assert!(
        f.controller.prepare_turn("thread-b").await.is_err(),
        "validation silently picked a fallback without applying it"
    );
    assert_eq!(f.count("thread/settings/update"), 1);
    f.close().await;
}

#[tokio::test]
async fn native_manual_change_with_lost_observation_prevents_later_restore() {
    let f = Fixture::new().await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"suppress_observation":true})).await;
    let result = f
        .executor
        .execute(
            CommandAction::Settings {
                reference: Some("thread-b".into()),
                model: Some("model-b".into()),
                effort: Some("medium".into()),
                speed: None,
            },
            42,
            3,
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        reserve_policy::get(&f.db, "thread-b")
            .unwrap()
            .unwrap()
            .mode,
        "manual"
    );
    f.configure(json!({"ordinary":true})).await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    assert_eq!(f.count("thread/settings/update"), 2);
    assert_eq!(f.rpc("thread/resume", json!({})).await["model"], "model-b");
    f.close().await;
}

#[tokio::test]
async fn native_null_settings_round_trip_and_wrong_thread_rejection() {
    let f = Fixture::new().await;
    f.configure(json!({"settings":{"model":"model-a","effort":null,"serviceTier":null}}))
        .await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    f.configure(json!({"ordinary":true})).await;
    f.controller.prepare_turn("thread-b").await.unwrap();
    let current = f.rpc("thread/resume", json!({})).await;
    assert!(current["reasoningEffort"].is_null());
    assert!(current["serviceTier"].is_null());
    f.configure(json!({"ordinary":false,"wrong_thread":true}))
        .await;
    assert!(f.controller.prepare_turn("thread-b").await.is_err());
    assert_eq!(f.count("thread/settings/update"), 2);
    f.close().await;
}

#[test]
fn wire_usage_variants_survive_terminal_journal_but_prose_and_rate_limit_do_not() {
    for spelling in [
        "usageLimitExceeded",
        "UsageLimitExceeded",
        "usage_limit_exceeded",
    ] {
        let event = json!({"threadId":"t","turn":{"id":"a","status":"failed", "error":{"message":"failed", "codexErrorInfo":spelling}}});
        let parsed = parse_turn_completion(&event, false).unwrap();
        assert!(parsed.usage_limit, "missed wire enum {spelling}");
        let text = serde_json::to_string(&event).unwrap();
        assert!(
            parse_turn_completion(&serde_json::from_str::<Value>(&text).unwrap(), false)
                .unwrap()
                .usage_limit
        );
    }
    for error in [
        json!({"message":"usage_limit_exceeded"}),
        json!({"codexErrorInfo":"rateLimitExceeded"}),
        json!({"data":{"code":"rate_limit_exceeded"}}),
    ] {
        assert!(!cdr_app_server::is_usage_limit_error(Some(&error)));
    }
}

#[test]
fn nonfailed_terminal_never_requests_a_usage_transition() {
    let value = json!({"threadId":"t","turn":{"id":"a","status":"completed","usageLimit":true,"error":{"message":"stale","codexErrorInfo":"usage_limit_exceeded"}}});
    assert!(!parse_turn_completion(&value, false).unwrap().usage_limit);
}

#[test]
fn old_policy_schema_is_detected_and_upgraded_before_reading_new_columns() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let conn = cdr_store::schema::open_initialized(&db).unwrap();
    conn.execute_batch("DROP TABLE codex_reserve_policy; CREATE TABLE codex_reserve_policy(thread_id TEXT PRIMARY KEY,mode TEXT NOT NULL,state TEXT NOT NULL,account_id TEXT,process_id INTEGER,generation INTEGER,previous_model TEXT,previous_effort TEXT,previous_tier TEXT,revision INTEGER NOT NULL DEFAULT 0,updated_at REAL NOT NULL DEFAULT 0);").unwrap();
    assert!(
        !reserve_policy::schema_current(&conn).unwrap(),
        "old table was incorrectly treated as current"
    );
    drop(conn);
    assert_eq!(reserve_policy::ensure(&db, "t").unwrap().state, "ordinary");
}

mod boundaries;

mod notices;

mod restart_notice;

mod revision5;

#[path = "revision14_tests.rs"]
mod revision14_tests;

#[path = "effort_policy_tests.rs"]
mod effort_policy_tests;

#[path = "entry_validation_tests.rs"]
mod entry_validation_tests;

#[path = "ordinary_reserve_admission_tests.rs"]
mod ordinary_reserve_admission_tests;

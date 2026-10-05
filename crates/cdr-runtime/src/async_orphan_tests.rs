use crate::queue_runner::{
    BackendFailure, BackendFailureKind, BoxBackendFuture, QueueCoordinator, TurnBackend, TurnRecord,
};
use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_store::{async_question as aq, queue};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[path = "../../../tests/support/submitted_successor_fixture.rs"]
pub(crate) mod submitted_fixture;
use submitted_fixture::original as fixture;

mod history;
mod submitted_successor;

#[tokio::test]
async fn orphan_actual_write_journal_reports_hold_without_sending_resume() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let mut config = crate::test_support::native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    let id = fixture::dispatching(&db, server.instance_id());
    crate::idle_release::install(&server, &db).unwrap();
    queue::complete(&db, "origin").unwrap();
    let error = server
        .execute(
            AppRequest {
                method: "thread/resume",
                params: json!({"threadId":"thread-b"}),
                timeout: Duration::from_secs(2),
            },
            Some(1),
        )
        .await
        .unwrap_err();
    server.close().await.unwrap();
    let trace = std::fs::read_to_string(temp.path().join("rpc.jsonl")).unwrap_or_default();
    assert!(
        !trace.lines().any(|line| line.contains("thread/resume")),
        "{trace}"
    );
    assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    assert!(
        error
            .to_string()
            .contains("[cdr-rust:async-resolution-held:v1]"),
        "{error}"
    );
}

struct HeldBackend {
    failure: BackendFailure,
    resumes: std::sync::atomic::AtomicUsize,
    starts: std::sync::atomic::AtomicUsize,
}

impl TurnBackend for HeldBackend {
    fn generation(&self) -> u64 {
        2
    }
    fn active_turn_id<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
    fn read_turns<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, Vec<TurnRecord>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn resume_thread<'a>(&'a self, _: &'a str) -> BoxBackendFuture<'a, ()> {
        Box::pin(async move {
            self.resumes
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(self.failure.clone())
        })
    }
    fn start_turn<'a>(&'a self, _: &'a str, _: &'a str) -> BoxBackendFuture<'a, String> {
        Box::pin(async move {
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(BackendFailure::ambiguous("unexpected start"))
        })
    }
}

#[tokio::test]
async fn cold_new_generation_does_not_start_or_resume_an_orphan_target() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    fixture::dispatching(&db, "old-resident");
    queue::complete(&db, "origin").unwrap();
    fixture::pending(&db, "next", "thread-b", 2);
    let before = queue::list(&db).unwrap();
    let backend = Arc::new(HeldBackend {
        failure: BackendFailure::definite("unexpected preflight"),
        resumes: 0.into(),
        starts: 0.into(),
    });
    for _ in 0..3 {
        let cold = QueueCoordinator::new(db.clone(), Arc::clone(&backend));
        cold.kick_target("thread-b").await.unwrap();
    }
    assert_eq!(queue::list(&db).unwrap(), before);
    assert_eq!(backend.resumes.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(backend.starts.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn actual_orphan_hold_is_not_presented_as_automatic_retry() {
    use crate::action_executor::{ActionContext, ActionExecutor};
    use crate::bridge_state::BridgeState;
    use crate::command_plan::CommandAction;
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("orphan.sqlite");
    let mut config = crate::test_support::native_fixture::config("async-question");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
    fixture::dispatching(&db, server.instance_id());
    crate::idle_release::install(&server, &db).unwrap();
    queue::complete(&db, "origin").unwrap();
    let native = crate::app_backend::AppServerTurnBackend::new(Arc::clone(&server));
    let failure = native.resume_thread("thread-b").await.unwrap_err();
    server.close().await.unwrap();
    assert_eq!(failure.kind, BackendFailureKind::ExecutionHeld);
    assert!(!failure.ambiguous);

    // Feed the actual connected backend classification through the public action
    // path; this fixture never sends Discord messages or a user turn.
    let state = temp.path().join("codex.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("../tests/fixtures/action_state.sql"))
        .unwrap();
    let mirror = temp.path().join("action.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(HeldBackend {
        failure,
        resumes: 0.into(),
        starts: 0.into(),
    });
    let queue = Arc::new(QueueCoordinator::new(mirror.clone(), Arc::clone(&backend)));
    let executor = ActionExecutor::new(state, mirror, bridge, queue);
    let result = executor
        .execute_with_context(
            CommandAction::Ask {
                prompt: "keep this new request".into(),
            },
            ActionContext {
                channel_id: 10,
                user_id: 20,
                discord_message_id: Some(30),
                auto_queue_when_busy: false,
            },
        )
        .await;
    let text = match result {
        Ok(result) => result.text,
        Err(error) => error.to_string(),
    };
    assert!(!text.contains("queued for automatic retry"), "{text}");
    assert!(text.contains("held"), "{text}");
    assert_eq!(backend.starts.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn certified_successor_archive_fence_reaches_actual_journal_before_transport() {
    for fenced in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let mut config = crate::test_support::native_fixture::config("async-question");
        config.environment.insert(
            "CDR_ACTION_RPC_LOG".into(),
            log.to_string_lossy().into_owned(),
        );
        let server = Arc::new(ResidentAppServer::start(config).await.unwrap());
        let id = fixture::dispatching(&db, server.instance_id());
        cdr_store::observed_completion::record_for_resident(
            &db,
            "thread-b",
            "original",
            1,
            r#"{"threadId":"thread-b","turn":{"id":"original","status":"completed"}}"#,
            server.instance_id(),
        )
        .unwrap();
        queue::mark_goal_waiting(&db, "origin", "original", 1).unwrap();
        let waiting = queue::list(&db).unwrap().remove(0);
        assert!(queue::attach_goal_turn_observed_if_owned(&db, &waiting, "successor", 1).unwrap());
        crate::idle_release::install(&server, &db).unwrap();
        if fenced {
            cdr_store::schema::open_initialized(&db)
                .unwrap()
                .execute(
                    "INSERT INTO codex_archive_fences(target_thread_id,operation_id,phase)
                 VALUES('thread-b','archive-intent','attempted')",
                    [],
                )
                .unwrap();
        }
        let before = queue::list(&db).unwrap();
        let result = server
            .execute(
                AppRequest {
                    method: "thread/resume",
                    params: json!({"threadId":"thread-b"}),
                    timeout: Duration::from_secs(2),
                },
                Some(1),
            )
            .await;
        server.close().await.unwrap();
        let trace = std::fs::read_to_string(&log).expect("native fixture RPC evidence");
        let resumes = trace
            .lines()
            .filter(|line| line.contains("thread/resume"))
            .count();
        if fenced {
            assert_eq!(resumes, 0, "fenced successor sent a mutation: {trace}");
            let error = result.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("mapping changed or target is fenced"),
                "{error}"
            );
            assert!(cdr_store::archive_fence::target_is_fenced(&db, "thread-b").unwrap());
        } else {
            // This question fixture does not implement resume. Its explicit
            // server rejection plus one logged frame proves the write occurred.
            assert!(
                matches!(
                    &result,
                    Err(cdr_app_server::AppServerError::Remote { code: -32601, .. })
                ),
                "{result:?}"
            );
            assert_eq!(
                resumes, 1,
                "positive transport control did not run: {trace}"
            );
        }
        assert_eq!(queue::list(&db).unwrap(), before);
        assert_eq!(aq::get(&db, &id).unwrap().state, "dispatching");
    }
}

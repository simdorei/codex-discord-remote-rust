use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::sync::Arc;
#[path = "support/resume_app_server.rs"]
mod support;

async fn scenario(name: &str, expected: Option<&str>, resumes: usize, reads: usize) {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(support::start(&temp, &log, name).await);
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let queue = Arc::new(QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    ));
    let executor =
        ActionExecutor::new(state, db.clone(), bridge.clone(), queue).with_server(server.clone());
    let result = executor
        .execute(
            CommandAction::Resume {
                reference: Some("thread-b".into()),
            },
            99,
            20,
        )
        .await;
    if let Some(expected) = expected {
        let message = result.unwrap().text;
        assert!(message.contains(expected), "{name}: {message}");
        assert!(message.contains("No prompt was resent"));
        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("thread-b")
        );
    } else {
        assert!(result.is_err(), "{name} cannot be verified resume success");
        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("thread-a")
        );
    }
    let calls = support::calls(&log);
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "thread/resume")
            .count(),
        resumes,
        "{name}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "thread/read")
            .count(),
        reads,
        "{name}"
    );
    for call in calls.iter().filter(|v| v["method"] != "initialize") {
        assert!(
            matches!(
                call["method"].as_str(),
                Some("thread/read" | "thread/resume")
            ),
            "{call}"
        );
        assert_eq!(call["params"]["threadId"], "thread-b");
    }
    assert!(cdr_store::queue::list(&db).unwrap().is_empty());
    server.close().await.unwrap();
}

#[tokio::test]
async fn already_loaded_is_read_only_and_never_resends() {
    scenario("idle", Some("already loaded"), 0, 1).await;
    scenario("active", Some("already loaded"), 0, 1).await;
}

#[tokio::test]
async fn not_loaded_is_resumed_once_and_verified_again() {
    scenario("recover", Some("recovered"), 1, 2).await;
}

#[tokio::test]
async fn missing_wrong_or_unhealthy_state_is_not_success() {
    for name in [
        "missing_status",
        "unknown_status",
        "systemError",
        "wrong_read",
        "conflicting_read",
        "missing_nested_read",
    ] {
        scenario(name, None, 0, 1).await;
    }
    scenario("wrong_resume", None, 1, 1).await;
    scenario("conflicting_resume", None, 1, 1).await;
    scenario("missing_nested_resume", None, 1, 1).await;
    scenario("still_unloaded", None, 1, 2).await;
    scenario("writer", None, 1, 1).await;
}

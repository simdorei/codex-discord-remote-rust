use cdr_runtime::{
    action_executor::ActionExecutor,
    app_backend::AppServerTurnBackend,
    bridge_state::BridgeState,
    command_plan::CommandAction,
    queue_runner::{QueueCoordinator, TurnBackend},
};
use std::sync::Arc;

#[path = "support/action_app_server.rs"]
mod support;

#[tokio::test]
async fn new_first_turn_does_not_resume_or_read_a_nonexistent_rollout() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(support::start_fake_server(&temp, &log).await);
    let backend = Arc::new(AppServerTurnBackend::new(server.clone()));
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        Arc::new(QueueCoordinator::new(db.clone(), backend.clone())),
    )
    .with_server(server.clone());
    let result = executor
        .execute(
            CommandAction::New {
                prompt: "첫 요청 확인".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    assert_eq!(result.text, "In progress\nmessage: 첫 요청 확인");
    let calls = support::rpc_log(&log);
    assert_eq!(
        calls
            .iter()
            .filter(|c| c["method"] == "thread/start")
            .count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|c| c["method"] == "turn/start").count(),
        1
    );
    assert!(!calls.iter().any(|c| c["method"] == "thread/resume"
        || c["method"] == "thread/read"
        || c["method"] == "thread/fork"));
    assert_eq!(
        cdr_store::queue::list(&db).unwrap()[0].turn_id.as_deref(),
        Some("first-turn")
    );
    // Once a start was attempted, the empty baseline can no longer bypass history.
    let error = backend.resume_thread("new-thread").await.unwrap_err();
    assert!(error.message.contains("no rollout found"));
    server.close().await.unwrap();
}

#[tokio::test]
async fn missing_history_without_a_confirmed_new_thread_is_not_ignored_or_recreated() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(support::start_fake_server(&temp, &log).await);
    let backend = AppServerTurnBackend::new(server.clone());
    assert!(
        backend
            .resume_thread("new-thread")
            .await
            .unwrap_err()
            .message
            .contains("no rollout found")
    );
    assert!(
        backend
            .read_turns("new-thread")
            .await
            .unwrap_err()
            .message
            .contains("no rollout found")
    );
    assert!(
        !support::rpc_log(&log)
            .iter()
            .any(|c| c["method"] == "turn/start" || c["method"] == "thread/start")
    );
    server.close().await.unwrap();
}

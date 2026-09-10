use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn resume_deadline_includes_writer_wait_and_cancels_before_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let release = temp.path().join("release");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        bridge.clone(),
        Arc::new(QueueCoordinator::new(
            db,
            Arc::new(AppServerTurnBackend::new(server.clone())),
        )),
    )
    .with_server(server.clone())
    .with_app_server_resume_timeout(Duration::from_millis(100));
    server
        .request(
            "test/pauseReads",
            json!({"releasePath":release}),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
    let mut blocked = Box::pin(server.request(
        "test/blocked",
        json!({"padding":"x".repeat(1_048_576)}),
        Duration::from_secs(5),
        None,
    ));
    assert!(futures_util::poll!(blocked.as_mut()).is_pending());
    let observed = tokio::time::timeout(
        Duration::from_millis(500),
        executor.execute(CommandAction::Resume { reference: None }, 42, 3),
    )
    .await;
    std::fs::write(&release, b"release").unwrap();
    tokio::time::timeout(Duration::from_secs(3), blocked)
        .await
        .unwrap()
        .unwrap();
    server.close().await.unwrap();
    let error = observed
        .expect("resume must finish while the transport writer is blocked")
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(!bridge.path().exists());
    let calls = std::fs::read_to_string(log).unwrap();
    for method in ["thread/read", "thread/resume", "thread/fork", "turn/start"] {
        assert!(
            !calls.contains(method),
            "cancelled resume dispatched {method}"
        );
    }
}

use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn options_timeout_includes_writer_wait_and_never_dispatches_after_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let release = temp.path().join("release");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    let executor = ActionExecutor::new(
        temp.path().join("unused-state.sqlite"),
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
    // A first poll acquires the free writer and stalls writing to the paused peer.
    assert!(futures_util::poll!(blocked.as_mut()).is_pending());
    let observed = tokio::time::timeout(
        Duration::from_millis(500),
        executor.execute(
            CommandAction::SettingsOptions {
                reference: None,
                field: Some("model".into()),
            },
            42,
            3,
        ),
    )
    .await;
    std::fs::write(&release, b"release").unwrap();
    tokio::time::timeout(Duration::from_secs(3), blocked)
        .await
        .unwrap()
        .unwrap();
    server.close().await.unwrap();
    let result = observed.expect("options must finish before the writer is released");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("settings options timed out")
    );
    assert!(!bridge.path().exists());
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(
        !calls.contains("model/list"),
        "cancelled lookup was dispatched later"
    );
    assert!(!calls.contains("thread/settings/update") && !calls.contains("thread/resume"));
}

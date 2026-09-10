use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn actual_update_does_not_accept_matching_observation_while_waiting_to_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let server = Arc::new(app::start(&temp, &log, "watermark").await);
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
    .with_server(server.clone());
    let mut action = Box::pin(executor.execute(
        CommandAction::Settings {
            reference: None,
            model: Some("model-b".into()),
            effort: None,
            speed: None,
        },
        42,
        3,
    ));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            assert!(futures_util::poll!(action.as_mut()).is_pending());
            if temp.path().join("resume-ready").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut blocked = Box::pin(server.request(
        "test/blocked",
        json!({"padding":"x".repeat(1_048_576)}),
        Duration::from_secs(5),
        None,
    ));
    assert!(futures_util::poll!(blocked.as_mut()).is_pending());
    std::fs::write(temp.path().join("resume-release"), b"release").unwrap();
    wait_model(&server, "model-a").await;
    // Initial model-a notification follows the resume ACK in the same pipe.
    // The next poll progresses to update writer acquisition, already held above.
    assert!(futures_util::poll!(action.as_mut()).is_pending());
    std::fs::write(temp.path().join("premature-emit"), b"emit").unwrap();
    wait_model(&server, "model-b").await;
    std::fs::write(temp.path().join("writer-release"), b"release").unwrap();
    tokio::time::timeout(Duration::from_secs(3), blocked)
        .await
        .unwrap()
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), action)
        .await
        .unwrap();
    server.close().await.unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outcome unverified")
    );
    assert!(!bridge.path().exists());
    let calls = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("thread/settings/update"))
            .count(),
        1
    );
    assert!(!calls.contains("thread/fork") && !calls.contains("turn/start"));
}

async fn wait_model(server: &cdr_app_server::ResidentAppServer, model: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if server
                .observed_thread_settings("thread-b", server.generation())
                .unwrap()
                .is_some_and(|(_, v)| v["model"] == model)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

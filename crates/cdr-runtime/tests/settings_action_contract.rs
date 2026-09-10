use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;

fn executor(
    temp: &tempfile::TempDir,
    server: Arc<cdr_app_server::ResidentAppServer>,
) -> (ActionExecutor<AppServerTurnBackend>, Arc<BridgeState>) {
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    (
        ActionExecutor::new(
            state,
            db.clone(),
            bridge.clone(),
            Arc::new(QueueCoordinator::new(
                db,
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server),
        bridge,
    )
}

#[tokio::test]
async fn query_reads_observed_settings_without_loading_or_changing_a_thread() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let (executor, bridge) = executor(&temp, server.clone());
    server
        .request(
            "test/observe",
            serde_json::json!({"threadId":"thread-b"}),
            Duration::from_secs(2),
            None,
        )
        .await
        .unwrap();
    let result = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: None,
                effort: None,
                speed: None,
            },
            42,
            3,
        )
        .await;
    server.close().await.unwrap();
    let result = result.unwrap();
    assert!(
        result.text.contains("model-a")
            && result.text.contains("high")
            && result.text.contains("standard")
    );
    assert!(!result.waits_for_final);
    assert!(!bridge.path().exists());
    let rpc = std::fs::read_to_string(log).unwrap();
    assert!(
        !rpc.contains("thread/resume")
            && !rpc.contains("thread/settings/update")
            && !rpc.contains("thread/fork")
    );
}

#[tokio::test]
async fn mismatched_application_is_not_reported_or_cached_as_a_success() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "mismatch").await);
    let (executor, bridge) = executor(&temp, server.clone());
    let result = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: Some("Model B".into()),
                effort: None,
                speed: None,
            },
            42,
            3,
        )
        .await;
    server.close().await.unwrap();
    assert!(
        result.is_err(),
        "empty RPC acknowledgement is not applied-settings evidence"
    );
    assert!(!bridge.path().exists());
    let rpc = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        rpc.lines()
            .filter(|line| line.contains("thread/settings/update"))
            .count(),
        1
    );
    assert!(!rpc.contains("thread/fork") && !rpc.contains("turn/start"));
}

#[tokio::test]
async fn verified_update_keeps_the_short_model_notice_and_cache() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let (executor, bridge) = executor(&temp, server.clone());
    let result = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: Some("Model B".into()),
                effort: None,
                speed: None,
            },
            42,
            3,
        )
        .await
        .unwrap();
    assert_eq!(result.text, "모델이 변경되었습니다: model-b");
    assert_eq!(
        bridge.thread_settings("thread-b").unwrap().model.as_deref(),
        Some("model-b")
    );
    server.close().await.unwrap();
    let rpc = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        rpc.lines()
            .filter(|line| line.contains("thread/settings/update"))
            .count(),
        1
    );
    assert!(!rpc.contains("thread/fork") && !rpc.contains("turn/start"));
}

#[tokio::test]
async fn unsupported_effort_is_rejected_for_the_actual_model() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "normal").await);
    let (executor, bridge) = executor(&temp, server.clone());
    let result = executor
        .execute(
            CommandAction::Settings {
                reference: None,
                model: None,
                effort: Some("medium".into()),
                speed: None,
            },
            42,
            3,
        )
        .await;
    assert!(result.unwrap_err().to_string().contains("not supported"));
    assert!(!bridge.path().exists());
    server.close().await.unwrap();
    assert!(
        !std::fs::read_to_string(log)
            .unwrap()
            .contains("thread/settings/update")
    );
}

#[tokio::test]
async fn missing_observation_cannot_confirm_even_an_acknowledged_update() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "missing").await);
    let (executor, bridge) = executor(&temp, server.clone());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        executor.execute(
            CommandAction::Settings {
                reference: None,
                model: Some("model-b".into()),
                effort: None,
                speed: None,
            },
            42,
            3,
        ),
    )
    .await
    .unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outcome unverified")
    );
    assert!(!bridge.path().exists());
    server.close().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|line| line.contains("thread/settings/update"))
            .count(),
        1
    );
}

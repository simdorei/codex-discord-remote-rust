use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use serde_json::Value;
use std::sync::Arc;
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn luna_reserve_selects_quota_alias_not_normal_luna_and_never_replays() {
    for name in ["reserve", "gpt-reserve", "Luna Reserve"] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, "reserve").await);
        let (executor, bridge) = setup(&temp, server.clone());
        let result = executor
            .execute(change(Some(name), Some("xhigh"), None), 42, 3)
            .await
            .unwrap();
        assert!(result.text.contains("Luna Reserve") && result.text.contains("gpt-reserve"));
        assert_eq!(
            bridge.thread_settings("thread-b").unwrap().model.as_deref(),
            Some("gpt-reserve")
        );
        assert_eq!(
            bridge.thread_settings("thread-b").unwrap().speed.as_deref(),
            Some("standard")
        );
        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("thread-a")
        );
        server.close().await.unwrap();
        let frames = frames(&log);
        let updates: Vec<_> = frames
            .iter()
            .filter(|v| v["method"] == "thread/settings/update")
            .collect();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0]["params"]["threadId"], "thread-b");
        assert_eq!(updates[0]["params"]["model"], "gpt-reserve");
        assert_eq!(updates[0]["params"]["effort"], "xhigh");
        assert_eq!(updates[0]["params"]["serviceTier"], "default");
        assert_no_turn(&frames);
    }
}

#[tokio::test]
async fn luna_reserve_invalid_quota_effort_or_fast_never_sends_update() {
    for (mode, effort, speed) in [
        ("normal", None, None),
        ("reserve-exhausted", None, None),
        ("reserve-metadata-missing", None, None),
        ("reserve", Some("max"), None),
        ("reserve", None, Some("fast")),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, mode).await);
        let (executor, bridge) = setup(&temp, server.clone());
        let before = std::fs::read(bridge.path()).unwrap();
        assert!(
            executor
                .execute(change(Some("reserve"), effort, speed), 42, 3)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
        server.close().await.unwrap();
        let frames = frames(&log);
        assert!(
            !frames
                .iter()
                .any(|v| v["method"] == "thread/settings/update")
        );
        assert_no_turn(&frames);
    }
}

#[tokio::test]
async fn luna_reserve_effort_only_retains_alias() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "reserve-active").await);
    let (executor, _) = setup(&temp, server.clone());
    executor
        .execute(change(None, Some("xhigh"), None), 42, 3)
        .await
        .unwrap();
    server.close().await.unwrap();
    let frames = frames(&log);
    let update = frames
        .iter()
        .find(|v| v["method"] == "thread/settings/update")
        .unwrap();
    assert_eq!(update["params"]["effort"], "xhigh");
    assert!(
        update["params"]
            .get("model")
            .is_none_or(|v| v == "gpt-reserve")
    );
    assert_no_turn(&frames);
}

#[tokio::test]
async fn luna_reserve_server_rejection_is_not_reported_or_saved_as_success() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "reserve-reject").await);
    let (executor, bridge) = setup(&temp, server.clone());
    let before = std::fs::read(bridge.path()).unwrap();
    let error = executor
        .execute(change(Some("reserve"), None, None), 42, 3)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("fixture settings rejection"));
    assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
    server.close().await.unwrap();
    let frames = frames(&log);
    assert_eq!(
        frames
            .iter()
            .filter(|v| v["method"] == "thread/settings/update")
            .count(),
        1
    );
    assert_no_turn(&frames);
}

#[tokio::test]
async fn luna_reserve_options_expose_alias_without_changing_selection() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "reserve").await);
    let (executor, bridge) = setup(&temp, server.clone());
    let before = std::fs::read(bridge.path()).unwrap();
    let result = executor
        .execute(
            CommandAction::SettingsOptions {
                reference: None,
                field: Some("model".into()),
            },
            42,
            3,
        )
        .await
        .unwrap();
    assert!(result.text.contains("gpt-reserve") && result.text.contains("Luna Reserve"));
    assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
    server.close().await.unwrap();
    let frames = frames(&log);
    assert!(!frames.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("thread/settings/update" | "thread/resume")
    )));
    assert_no_turn(&frames);
}

fn change(model: Option<&str>, effort: Option<&str>, speed: Option<&str>) -> CommandAction {
    CommandAction::Settings {
        reference: Some("thread-b".into()),
        model: model.map(str::to_owned),
        effort: effort.map(str::to_owned),
        speed: speed.map(str::to_owned),
    }
}
fn assert_no_turn(frames: &[Value]) {
    assert!(!frames.iter().any(|v| matches!(
        v["method"].as_str(),
        Some("turn/start" | "turn/steer" | "thread/start" | "thread/fork")
    )));
}
fn setup(
    temp: &tempfile::TempDir,
    server: Arc<cdr_app_server::ResidentAppServer>,
) -> (ActionExecutor<AppServerTurnBackend>, Arc<BridgeState>) {
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-a", "project", "title", 100, 42, 1.0).unwrap();
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
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
fn frames(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn luna_reserve_accepts_explicit_standard_with_effective_default_tier() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "reserve-default-tier").await);
    let (executor, bridge) = setup(&temp, server.clone());
    let result = executor
        .execute(change(Some("reserve"), Some("xhigh"), None), 42, 3)
        .await;
    server.close().await.unwrap();
    let result = result.expect("effective default is the requested explicit standard tier");
    assert!(result.text.contains("속도: standard"), "{}", result.text);
    assert!(result.text.contains("gpt-reserve"));
    let stored = bridge.thread_settings("thread-b").unwrap();
    assert_eq!(stored.model.as_deref(), Some("gpt-reserve"));
    assert_eq!(stored.speed.as_deref(), Some("standard"));
    let frames = frames(&log);
    let updates: Vec<_> = frames
        .iter()
        .filter(|v| v["method"] == "thread/settings/update")
        .collect();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["params"]["model"], "gpt-reserve");
    assert_eq!(updates[0]["params"]["effort"], "xhigh");
    assert_eq!(updates[0]["params"]["serviceTier"], "default");
    assert_no_turn(&frames);
}

#[tokio::test]
async fn luna_reserve_mismatched_effective_fields_remain_unverified_without_retry() {
    for mode in [
        "reserve-tier-mismatch",
        "reserve-model-mismatch",
        "reserve-effort-mismatch",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, mode).await);
        let (executor, bridge) = setup(&temp, server.clone());
        let before = std::fs::read(bridge.path()).unwrap();
        let result = executor
            .execute(change(Some("reserve"), Some("xhigh"), None), 42, 3)
            .await;
        server.close().await.unwrap();
        let error = result.expect_err("a genuinely different effective setting is not success");
        assert!(error.to_string().contains("observed values do not match"));
        assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
        let frames = frames(&log);
        assert_eq!(
            frames
                .iter()
                .filter(|v| v["method"] == "thread/settings/update")
                .count(),
            1
        );
        assert_no_turn(&frames);
    }
}

use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use serde_json::Value;
use std::sync::Arc;
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn settings_noop_uses_fresh_resume_without_update_or_replay() {
    for (mode, model, effort) in [
        ("reserve-noop", "reserve", "xhigh"),
        ("normal", "model-a", "high"),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, mode).await);
        let (executor, bridge) = setup(&temp, server.clone());
        let result = executor.execute(change(model, effort), 42, 3).await;
        server.close().await.unwrap();
        let result = result.expect("an already-matching fresh server snapshot needs no update");
        assert!(
            result.text.contains("이미 적용된 설정 확인"),
            "{}",
            result.text
        );
        assert!(
            result.text.contains("변경 요청을 보내지 않음"),
            "{}",
            result.text
        );
        let expected = if model == "reserve" {
            "gpt-reserve"
        } else {
            model
        };
        assert_eq!(
            bridge.thread_settings("thread-b").unwrap().model.as_deref(),
            Some(expected)
        );
        assert_eq!(
            bridge.selected_thread_id().unwrap().as_deref(),
            Some("thread-a")
        );
        let frames = frames(&log);
        assert_eq!(count(&frames, "thread/resume"), 1);
        assert_eq!(count(&frames, "thread/settings/update"), 0);
        assert_no_turn(&frames);
    }
}

#[tokio::test]
async fn settings_noop_does_not_trust_stale_local_values_or_ack_alone() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log, "reserve-missing-observation").await);
    let (executor, bridge) = setup(&temp, server.clone());
    bridge
        .remember_thread_settings(
            "thread-b",
            Some("gpt-reserve"),
            Some("xhigh"),
            Some("standard"),
        )
        .unwrap();
    let before = std::fs::read(bridge.path()).unwrap();
    let result = executor.execute(change("reserve", "xhigh"), 42, 3).await;
    server.close().await.unwrap();
    let error = result.expect_err("a real update without an observation remains unverified");
    assert!(
        error
            .to_string()
            .contains("no fresh matching settings observation arrived")
    );
    assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
    let frames = frames(&log);
    assert_eq!(count(&frames, "thread/resume"), 1);
    assert_eq!(count(&frames, "thread/settings/update"), 1);
    assert_no_turn(&frames);
}

#[tokio::test]
async fn settings_noop_rejects_wrong_thread_and_incomplete_resume() {
    for mode in ["reserve-noop-wrong-thread", "reserve-noop-incomplete"] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, mode).await);
        let (executor, bridge) = setup(&temp, server.clone());
        let before = std::fs::read(bridge.path()).unwrap();
        let result = executor.execute(change("reserve", "xhigh"), 42, 3).await;
        server.close().await.unwrap();
        assert!(
            result.is_err(),
            "an incomplete or foreign snapshot is not confirmation"
        );
        assert_eq!(std::fs::read(bridge.path()).unwrap(), before);
        let frames = frames(&log);
        assert_eq!(count(&frames, "thread/settings/update"), 0);
        assert_no_turn(&frames);
    }
}

fn change(model: &str, effort: &str) -> CommandAction {
    CommandAction::Settings {
        reference: Some("thread-b".into()),
        model: Some(model.into()),
        effort: Some(effort.into()),
        speed: None,
    }
}

fn count(frames: &[Value], method: &str) -> usize {
    frames
        .iter()
        .filter(|value| value["method"] == method)
        .count()
}

fn assert_no_turn(frames: &[Value]) {
    assert!(!frames.iter().any(|value| matches!(
        value["method"].as_str(),
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

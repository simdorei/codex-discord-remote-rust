use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/archive_app_server.rs"]
mod support;

async fn deadline_scenario(name: &str, expected_phase: &str, archive_calls: usize) {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    let server = Arc::new(support::start(&temp, &state, name).await);
    let db = temp.path().join("mirror.sqlite");
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let executor = ActionExecutor::new(
        state.clone(),
        db.clone(),
        bridge.clone(),
        Arc::new(QueueCoordinator::new(
            db,
            Arc::new(AppServerTurnBackend::new(server.clone())),
        )),
    )
    .with_server(server.clone())
    .with_app_server_resume_timeout(Duration::from_millis(300));
    let observed = tokio::time::timeout(
        Duration::from_secs(2),
        executor.execute(CommandAction::Archive { reference: None }, 99, 20),
    )
    .await;
    // Always stop the disposable resident, including on the expected RED path.
    server.close().await.unwrap();
    let error = observed
        .expect("archive must honor its total operation deadline")
        .expect_err("a timed-out archive is not verified success")
        .to_string();
    assert!(error.contains(expected_phase), "{error}");
    assert_eq!(
        bridge.selected_thread_id().unwrap().as_deref(),
        Some("thread-b")
    );
    let calls = support::calls(&temp);
    assert_eq!(
        calls
            .iter()
            .filter(|v| v["method"] == "thread/archive")
            .count(),
        archive_calls
    );
    assert!(
        !calls
            .iter()
            .any(|v| matches!(v["method"].as_str(), Some("thread/fork" | "turn/start")))
    );
    assert_eq!(
        cdr_codex_state::CodexThreadStore::open(&state)
            .unwrap()
            .load_thread("thread-b", true)
            .unwrap()
            .is_some(),
        archive_calls == 1
    );
}

#[tokio::test]
async fn scope_lookup_timeout_does_not_send_archive() {
    deadline_scenario("stall_list", "no archive was sent", 0).await;
}

#[tokio::test]
async fn timeout_after_archive_dispatch_preserves_uncertain_outcome_without_retry() {
    deadline_scenario("stall_archive", "outcome is unverified", 1).await;
}

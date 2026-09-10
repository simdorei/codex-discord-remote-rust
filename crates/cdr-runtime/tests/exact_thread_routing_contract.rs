use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use cdr_store::mapping::{mirrored_thread_id, upsert_thread};
use std::sync::Arc;

#[path = "support/action_app_server.rs"]
mod support;

async fn check_target(target: &str, conflict: bool) {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(support::start_fake_server(&temp, &log).await);
    let db = temp.path().join("mirror.sqlite");
    let state = temp.path().join("state.sqlite");
    rusqlite::Connection::open(&state)
        .unwrap()
        .execute_batch(include_str!("fixtures/action_state.sql"))
        .unwrap();
    upsert_thread(&db, target, "project", "Original", 98, 99, 1.0).unwrap();
    let queue = Arc::new(QueueCoordinator::new(
        db.clone(),
        Arc::new(AppServerTurnBackend::new(server.clone())),
    ));
    let executor = ActionExecutor::new(
        state,
        db.clone(),
        Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
        queue.clone(),
    )
    .with_server(server.clone());
    let result = executor
        .execute(
            CommandAction::Ask {
                prompt: "same conversation".into(),
            },
            99,
            20,
        )
        .await;
    assert!(
        result.is_ok(),
        "request must remain accepted on original ID: {result:?}"
    );
    let jobs = cdr_store::queue::list(&db).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].target_thread_id, target);
    assert_eq!(
        mirrored_thread_id(&db, Some(99)).unwrap().as_deref(),
        Some(target)
    );
    if conflict {
        assert!(result.unwrap().text.contains("active writer"));
        assert!(jobs[0].turn_id.is_none());
    } else {
        assert_eq!(jobs[0].turn_id.as_deref(), Some("existing-turn"));
    }
    queue.recover().await.unwrap();
    let calls = support::rpc_log(&log);
    assert!(
        !calls
            .iter()
            .any(|c| c["method"] == "thread/fork" || c["method"] == "thread/start")
    );
    assert!(
        calls
            .iter()
            .filter(|c| c["params"].get("threadId").is_some())
            .all(|c| c["params"]["threadId"] == target)
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn existing_mirror_uses_exact_id_without_copy() {
    check_target("thread-b", false).await;
}

#[tokio::test]
async fn writer_conflict_preserves_request_without_copy_even_during_recovery() {
    check_target("thread-a", true).await;
}

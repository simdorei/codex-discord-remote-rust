//! DG1: validate-to-send transitions preserve the original target and fail closed.
use cdr_app_server::requests::{AppRequest, steer_turn};
use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/action_app_server.rs"]
mod server_support;
#[path = "support/action_target.rs"]
mod support;

#[tokio::test]
async fn server_handoff_at_steer_receipt_preserves_wire_target_and_never_replays() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(support::FakeBackend::default());
    let executor = support::executor(
        &temp,
        temp.path().join("mirror.sqlite"),
        bridge,
        Arc::clone(&backend),
    )
    .with_server(Arc::clone(&server));
    for method in ["test/active-turn", "test/stale-next-steer"] {
        server
            .execute(
                AppRequest {
                    method,
                    params: json!({"threadId":"thread-a","turnId":"current"}),
                    timeout: Duration::from_secs(2),
                },
                None,
            )
            .await
            .unwrap();
    }
    let error = executor
        .execute(
            CommandAction::Steer {
                prompt: "keep target".into(),
            },
            99,
            20,
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("stale expectedTurnId"),
        "{error}"
    );
    assert_eq!(
        server.active_turn_id("thread-a").await.unwrap().as_deref(),
        Some("next")
    );
    let requests = server_support::rpc_log(&log);
    let steers: Vec<_> = requests
        .iter()
        .filter(|r| r["method"] == "turn/steer")
        .collect();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0]["params"]["expectedTurnId"], "current");
    assert_eq!(steers[0]["params"]["threadId"], "thread-a");
    assert!(!requests.iter().any(|r| matches!(
        r["method"].as_str(),
        Some("turn/start" | "thread/fork" | "thread/resume")
    )));
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    server.close().await.unwrap();
}

#[tokio::test]
async fn generation_replacement_after_validation_rejects_before_steer_wire() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(support::FakeBackend::default());
    let executor = support::executor(
        &temp,
        temp.path().join("mirror.sqlite"),
        bridge,
        Arc::clone(&backend),
    )
    .with_server(Arc::clone(&server));
    server
        .execute(
            AppRequest {
                method: "test/active-turn",
                params: json!({"threadId":"thread-a","turnId":"current"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    let (turn, generation) = executor
        .verified_control_turn(99, "thread-a", None)
        .await
        .unwrap();
    server
        .execute(
            AppRequest {
                method: "test/finish-turn",
                params: json!({"threadId":"thread-a","turnId":"current"}),
                timeout: Duration::from_secs(2),
            },
            None,
        )
        .await
        .unwrap();
    assert!(server.force_restart_if_quiescent().await.unwrap());
    assert_ne!(server.generation(), generation);
    let error = server
        .execute(
            steer_turn("thread-a", "stale request", &turn),
            Some(generation),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("generation"), "{error}");
    let requests = server_support::rpc_log(&log);
    assert!(!requests.iter().any(|r| matches!(
        r["method"].as_str(),
        Some("turn/steer" | "turn/start" | "thread/fork" | "thread/resume")
    )));
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    server.close().await.unwrap();
}

use cdr_app_server::requests::AppRequest;
use cdr_runtime::{bridge_state::BridgeState, command_plan::CommandAction};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn stop_uses_verified_original_turn_without_resume_or_idle_guess() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-b")).unwrap();
    let backend = Arc::new(support::FakeBackend::default());
    let executor =
        support::executor(&temp, db.clone(), bridge, backend).with_server(server.clone());
    let stop = CommandAction::Stop {
        reference: Some("thread-a".into()),
    };
    let result = executor.execute(stop.clone(), 99, 20).await;
    assert!(
        result.is_err(),
        "cache absence is unknown, not a successful no-active-turn result"
    );
    assert!(
        !server_support::rpc_log(&log)
            .iter()
            .any(|r| r["method"] == "thread/resume"),
        "stop must not acquire a writer as an undocumented fallback"
    );
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
    executor.execute(stop.clone(), 99, 20).await.unwrap();
    let generation = i64::try_from(server.generation()).unwrap();
    cdr_store::queue::enqueue(
        &db,
        cdr_store::queue::NewQueueJob {
            job_id: "job",
            target_thread_id: "thread-a",
            channel_id: 99,
            owner_user_id: Some(20),
            discord_message_id: None,
            app_server_generation: generation,
            prompt: "test",
            queued: false,
            ack_sent: true,
            created_at: 1.0,
        },
    )
    .unwrap();
    cdr_store::queue::begin_attempt(&db, "job", &[], generation).unwrap();
    cdr_store::queue::mark_running(&db, "job", "current", generation).unwrap();
    assert!(
        cdr_store::observed_completion::record(
            &db,
            "thread-a",
            "current",
            generation,
            r#"{"threadId":"thread-a","turn":{"id":"current","status":"completed"}}"#
        )
        .unwrap()
    );
    assert!(
        executor.execute(stop, 99, 20).await.is_err(),
        "a terminal observed on disk must revoke stop as well as steering"
    );
    let requests = server_support::rpc_log(&log);
    let interrupts = requests
        .iter()
        .filter(|r| r["method"] == "turn/interrupt")
        .collect::<Vec<_>>();
    assert_eq!(interrupts.len(), 1);
    assert_eq!(interrupts[0]["params"]["threadId"], "thread-a");
    assert_eq!(interrupts[0]["params"]["turnId"], "current");
    server.close().await.unwrap();
}

#[path = "support/action_app_server.rs"]
mod server_support;
#[path = "support/action_target.rs"]
mod support;

#[tokio::test]
async fn shared_control_rejects_unknown_stale_and_changed_target_without_starting_any_turn() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(support::FakeBackend::default());
    let executor = support::executor(&temp, db, Arc::clone(&bridge), Arc::clone(&backend))
        .with_server(Arc::clone(&server));
    let error = executor
        .verified_control_turn(99, "thread-a", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no currently owned active turn"));
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
    assert_eq!(
        executor
            .verified_control_turn(99, "thread-a", Some("current"))
            .await
            .unwrap()
            .0,
        "current"
    );
    assert!(
        executor
            .verified_control_turn(99, "thread-a", Some("old"))
            .await
            .unwrap_err()
            .to_string()
            .contains("original turn has ended")
    );
    bridge.set_selected_thread_id(Some("different")).unwrap();
    assert!(
        executor
            .verified_control_turn(99, "thread-a", Some("current"))
            .await
            .unwrap_err()
            .to_string()
            .contains("target changed")
    );
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(
        !server_support::rpc_log(&log)
            .iter()
            .any(|event| event["method"] == "turn/steer")
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn command_steer_sends_exact_expected_turn_and_never_forks() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(server_support::start_fake_server(&temp, &log).await);
    let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
    bridge.set_selected_thread_id(Some("thread-a")).unwrap();
    let backend = Arc::new(support::FakeBackend::default());
    let executor =
        support::executor(&temp, db, bridge, Arc::clone(&backend)).with_server(Arc::clone(&server));
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
    executor
        .execute(
            CommandAction::Steer {
                prompt: "change direction".into(),
            },
            99,
            20,
        )
        .await
        .unwrap();
    let log = server_support::rpc_log(&log);
    let steers: Vec<_> = log
        .iter()
        .filter(|event| event["method"] == "turn/steer")
        .collect();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0]["params"]["expectedTurnId"], "current");
    assert_eq!(steers[0]["params"]["threadId"], "thread-a");
    assert!(backend.forks.lock().await.is_empty());
    assert!(backend.starts.lock().await.is_empty());
    server.close().await.unwrap();
}

use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    interaction_worker::run_interaction_worker, queue_runner::QueueCoordinator,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;
#[path = "support/approval_http.rs"]
mod http;
#[path = "support/mapped_slash.rs"]
mod slash;

#[tokio::test]
async fn read_only_settings_admission_does_not_claim_a_mutation_binding() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
    let _work = slash::stage(&db, "settings").await;
    let record = cdr_store::ingress::get(&db, "interaction:101")
        .unwrap()
        .unwrap();
    assert!(record.payload["settings_binding"].is_null());
    assert!(record.target_thread_id.is_none());
}

#[tokio::test]
async fn slash_settings_keeps_admitted_original_target_across_mapping_and_selection_changes() {
    for route in ["mapped", "selected", "explicit", "selected-unchanged"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        let selected = route.starts_with("selected");
        cdr_store::mapping::upsert_thread(
            &db,
            "thread-b",
            "p",
            "b",
            100,
            if selected { 43 } else { 42 },
            1.0,
        )
        .unwrap();
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        bridge.set_selected_thread_id(Some("thread-b")).unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, "normal").await);
        let executor = Arc::new(
            ActionExecutor::new(
                state,
                db.clone(),
                bridge.clone(),
                Arc::new(QueueCoordinator::new(
                    db.clone(),
                    Arc::new(AppServerTurnBackend::new(server.clone())),
                )),
            )
            .with_server(server.clone()),
        );
        let mut options = vec![json!({"name":"model","type":3,"value":"Model B"})];
        if route == "explicit" {
            options.push(json!({"name":"ref","type":3,"value":"thread-b"}));
        }
        let work = slash::stage_options(&db, "settings", json!(options)).await;
        let original = cdr_store::ingress::get(&db, "interaction:101")
            .unwrap()
            .unwrap();
        assert_eq!(original.target_thread_id.as_deref(), Some("thread-b"));
        if route == "selected" {
            bridge.set_selected_thread_id(Some("thread-a")).unwrap();
        } else if route != "selected-unchanged" {
            cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 43, 2.0).unwrap();
            cdr_store::mapping::upsert_thread(&db, "thread-a", "p", "a", 100, 42, 2.0).unwrap();
        }
        let bridge_before_work = std::fs::read(bridge.path()).unwrap();
        let transport = http::start().await;
        let client = Arc::new(
            twilight_http::Client::builder()
                .proxy(transport.address, true)
                .ratelimiter(None)
                .build(),
        );
        let (send, receive) = tokio::sync::mpsc::channel(1);
        send.send(work).await.unwrap();
        drop(send);
        tokio::time::timeout(
            Duration::from_secs(5),
            run_interaction_worker(receive, executor, server.clone(), client),
        )
        .await
        .unwrap();
        transport.stop.send(()).unwrap();
        let traffic = transport.task.await.unwrap();
        server.close().await.unwrap();
        assert_eq!(traffic.len(), 1);
        let text = traffic[0].1["content"].as_str().unwrap();
        let succeeds = matches!(route, "explicit" | "selected-unchanged");
        if succeeds {
            assert_eq!(text, "모델이 변경되었습니다: model-b");
        } else {
            assert!(
                text.contains("settings target changed after admission"),
                "{text}"
            );
            assert_eq!(std::fs::read(bridge.path()).unwrap(), bridge_before_work);
        }
        let record = cdr_store::ingress::get(&db, "interaction:101")
            .unwrap()
            .unwrap();
        assert_eq!(record.payload, original.payload);
        assert_eq!(record.target_thread_id, original.target_thread_id);
        assert_original_calls(&log, succeeds);
    }
}

fn assert_original_calls(log: &std::path::Path, succeeds: bool) {
    let calls = std::fs::read_to_string(log).unwrap();
    let updates: Vec<serde_json::Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|call: &serde_json::Value| call["method"] == "thread/settings/update")
        .collect();
    assert_eq!(updates.len(), usize::from(succeeds));
    if succeeds {
        assert_eq!(updates[0]["params"]["threadId"], "thread-b");
    } else {
        assert!(!calls.contains("thread/resume"));
    }
    assert!(!calls.contains("thread/fork") && !calls.contains("turn/start"));
}

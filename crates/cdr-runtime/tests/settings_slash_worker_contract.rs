use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    interaction_worker::run_interaction_worker, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;
#[path = "support/approval_http.rs"]
mod http;
#[path = "support/mapped_slash.rs"]
mod slash;

#[tokio::test]
async fn actual_settings_slash_query_and_change_finish_with_verified_short_response() {
    for change in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        let state = temp.path().join("state.sqlite");
        rusqlite::Connection::open(&state)
            .unwrap()
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, "normal").await);
        server
            .request(
                "test/observe",
                serde_json::json!({"threadId":"thread-b"}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let executor = Arc::new(
            ActionExecutor::new(
                state,
                db.clone(),
                Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
                Arc::new(QueueCoordinator::new(
                    db.clone(),
                    Arc::new(AppServerTurnBackend::new(server.clone())),
                )),
            )
            .with_server(server.clone()),
        );
        let fixture = http::start().await;
        let client = Arc::new(
            twilight_http::Client::builder()
                .proxy(fixture.address, true)
                .ratelimiter(None)
                .build(),
        );
        let work = if change {
            slash::stage_options(
                &db,
                "settings",
                serde_json::json!([
            {"name":"model","type":3,"value":"Model B"}]),
            )
            .await
        } else {
            slash::stage(&db, "settings").await
        };
        let (send, recv) = tokio::sync::mpsc::channel(1);
        send.send(work).await.unwrap();
        drop(send);
        tokio::time::timeout(
            Duration::from_secs(5),
            run_interaction_worker(recv, executor, server.clone(), client),
        )
        .await
        .unwrap();
        fixture.stop.send(()).unwrap();
        let traffic = fixture.task.await.unwrap();
        assert_eq!(traffic.len(), 1);
        assert!(
            !traffic[0].0,
            "settings reply must complete the original slash response"
        );
        let text = traffic[0].1["content"].as_str().unwrap();
        if change {
            assert_eq!(text, "모델이 변경되었습니다: model-b");
        } else {
            assert!(text.contains("마지막 서버 확인값") && text.contains("model-a"));
        }
        assert_eq!(
            cdr_store::ingress::get(&db, "interaction:101")
                .unwrap()
                .unwrap()
                .state,
            "completed"
        );
        server.close().await.unwrap();
        let calls = std::fs::read_to_string(log).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("thread/settings/update"))
                .count(),
            usize::from(change)
        );
        if !change {
            assert!(!calls.contains("thread/resume"));
        }
        assert!(!calls.contains("turn/start") && !calls.contains("thread/fork"));
    }
}

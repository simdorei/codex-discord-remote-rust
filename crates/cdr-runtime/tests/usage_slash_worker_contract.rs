use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    interaction_worker::run_interaction_worker, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/usage_app_server.rs"]
mod app;
#[path = "support/approval_http.rs"]
mod http;
#[path = "support/mapped_slash.rs"]
mod slash;

#[tokio::test]
async fn actual_usage_slash_preserves_success_unavailable_and_rpc_errors() {
    for mode in ["normal", "empty", "rates_error", "usage_error"] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "p", "b", 100, 42, 1.0).unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(temp.path(), &log, mode).await);
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        let executor = Arc::new(
            ActionExecutor::new(
                temp.path().join("unused-state.sqlite"),
                db.clone(),
                bridge.clone(),
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
        let work = slash::stage(&db, "usage").await;
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
        assert!(!traffic[0].0, "complete original slash response");
        let text = traffic[0].1["content"].as_str().unwrap();
        match mode {
            "normal" => {
                assert!(text.contains("total_tokens: 1234"));
                assert!(text.contains("used=25% window=5h"));
                assert!(!text.contains("999999"));
            }
            "empty" => {
                assert!(text.contains("usage data unavailable"));
                assert!(!text.contains("total_tokens: 0"));
            }
            "rates_error" => assert!(text.contains("fixture rate lookup failed")),
            "usage_error" => assert!(text.contains("fixture usage lookup failed")),
            _ => unreachable!(),
        }
        assert!(!bridge.path().exists());
        server.close().await.unwrap();
        let calls = std::fs::read_to_string(log).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|v| v.contains("account/rateLimits/read"))
                .count(),
            1
        );
        assert_eq!(
            calls
                .lines()
                .filter(|v| v.contains("account/usage/read"))
                .count(),
            usize::from(mode != "rates_error")
        );
        assert!(!calls.contains("thread/") && !calls.contains("turn/"));
    }
}

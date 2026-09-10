use cdr_runtime::{
    action_executor::ActionExecutor, app_backend::AppServerTurnBackend, bridge_state::BridgeState,
    command_plan::CommandAction, queue_runner::QueueCoordinator,
};
use std::{sync::Arc, time::Duration};
#[path = "support/settings_app_server.rs"]
mod app;

#[tokio::test]
async fn effort_options_use_only_the_target_model_and_label_the_evidence_source() {
    for observed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state.sqlite");
        let connection = rusqlite::Connection::open(&state).unwrap();
        connection
            .execute_batch(include_str!("fixtures/action_state.sql"))
            .unwrap();
        connection
            .execute("UPDATE threads SET model='model-b' WHERE id='thread-b'", [])
            .unwrap();
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log, "normal").await);
        let bridge = Arc::new(BridgeState::new(temp.path().join("bridge.json")));
        let executor = ActionExecutor::new(
            state,
            db.clone(),
            bridge.clone(),
            Arc::new(QueueCoordinator::new(
                db,
                Arc::new(AppServerTurnBackend::new(server.clone())),
            )),
        )
        .with_server(server.clone());
        if observed {
            server
                .request(
                    "test/observe",
                    serde_json::json!({"threadId":"thread-b"}),
                    Duration::from_secs(2),
                    None,
                )
                .await
                .unwrap();
        }
        let result = executor
            .execute(
                CommandAction::SettingsOptions {
                    reference: Some("thread-b".into()),
                    field: Some("effort".into()),
                },
                42,
                3,
            )
            .await
            .unwrap();
        assert_eq!(
            result.text,
            if observed {
                "대화: thread-b\n마지막 서버 확인 모델: model-a\nhigh\nlow"
            } else {
                "대화: thread-b\n마지막 저장 모델 · 현재 실행값 미확인: model-b\nmedium"
            }
        );
        assert!(!bridge.path().exists());
        server.close().await.unwrap();
        let rpc = std::fs::read_to_string(log).unwrap();
        for mutation in [
            "thread/resume",
            "thread/settings/update",
            "thread/fork",
            "turn/start",
        ] {
            assert!(
                !rpc.contains(mutation),
                "option lookup must not issue {mutation}"
            );
        }
    }
}

use cdr_runtime::{
    action_ui::render_action_ui, bridge_state::BridgeState, command_plan::CommandAction,
};
use cdr_store::queue;
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[path = "support/approval_app_server.rs"]
mod app;
#[path = "support/action_target.rs"]
mod target;

#[tokio::test]
async fn approval_command_redisplays_existing_bound_buttons_without_resuming_or_answering() {
    for method in [
        "item/commandExecution/requestApproval",
        "item/tool/requestUserInput",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log).await);
        let db = temp.path().join("mirror.sqlite");
        cdr_store::mapping::upsert_thread(&db, "thread-b", "project", "title", 100, 42, 1.0)
            .unwrap();
        let executor = target::executor(
            &temp,
            db.clone(),
            Arc::new(BridgeState::new(temp.path().join("bridge.json"))),
            Arc::new(target::FakeBackend::default()),
        )
        .with_server(server.clone());
        queue::enqueue(
            &db,
            queue::NewQueueJob {
                job_id: "active-job",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(101),
                app_server_generation: i64::try_from(server.generation()).unwrap(),
                prompt: "original",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        cdr_store::schema::open_initialized(&db)
            .unwrap()
            .execute(
                "UPDATE codex_turn_queue SET state='running',turn_id='turn-b',attempt_count=1",
                [],
            )
            .unwrap();
        server
            .request(
                "test/pending",
                json!({"method":method}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let before = server.pending_server_requests(None).await.unwrap();
        let result = executor
            .execute(CommandAction::Approval, 42, 3)
            .await
            .unwrap();
        let after = server.pending_server_requests(None).await.unwrap();
        server.close().await.unwrap();
        assert_eq!(before, after, "redisplay does not make or answer a request");
        assert!(
            !render_action_ui(result.ui.as_ref()).unwrap().is_empty(),
            "{method}: {}",
            result.text
        );
        let calls = std::fs::read_to_string(log).unwrap();
        assert!(!calls.contains("thread/resume") && !calls.contains("thread/fork"));
        assert!(!calls.lines().any(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["id"] == "approval-1" && value.get("result").is_some()
        }));
    }
}

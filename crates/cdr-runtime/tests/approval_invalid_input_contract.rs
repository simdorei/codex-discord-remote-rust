use cdr_runtime::component_worker::handle_pending_text_reply;
use serde_json::json;
use std::time::Duration;
#[path = "support/approval_app_server.rs"]
mod app;

#[tokio::test]
async fn malformed_input_cannot_be_answered_through_text() {
    for questions in [
        json!([{"id":"q","question":"Choose","options":[{"label":""},{"label":"Second"}]}]),
        json!([{"id":"q","question":"Choose","options":[{}, {"label":"Second"}]}]),
        json!([{"id":"q","question":"Choose","options":false}]),
        json!([{"id":"q","question":"First","options":null},{"id":" q ","question":"Second","options":null}]),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("db.sqlite");
        let log = temp.path().join("rpc.jsonl");
        let server = app::start(&temp, &log).await;
        let generation = i64::try_from(server.generation()).unwrap();
        cdr_store::queue::enqueue(
            &db,
            cdr_store::queue::NewQueueJob {
                job_id: "owner",
                target_thread_id: "thread-b",
                channel_id: 42,
                owner_user_id: Some(3),
                discord_message_id: Some(101),
                app_server_generation: generation,
                prompt: "original",
                queued: false,
                ack_sent: true,
                created_at: 1.0,
            },
        )
        .unwrap();
        cdr_store::queue::begin_attempt(&db, "owner", &[], generation).unwrap();
        cdr_store::queue::mark_running(&db, "owner", "turn-b", generation).unwrap();
        server
            .request(
                "test/pending",
                json!({"method":"item/tool/requestUserInput","questions":questions}),
                Duration::from_secs(2),
                None,
            )
            .await
            .unwrap();
        let before = server.pending_server_requests(None).await.unwrap();
        let result = handle_pending_text_reply("thread-b", "1", &server, &db, 42, 3).await;
        let after = server.pending_server_requests(None).await.unwrap();
        server.close().await.unwrap();
        assert!(
            result.is_err(),
            "malformed request was answered: {questions}"
        );
        assert_eq!(after, before);
        assert!(!std::fs::read_to_string(log).unwrap().lines().any(|line| {
            let frame: serde_json::Value = serde_json::from_str(line).unwrap();
            frame["id"] == "approval-1" && frame.get("result").is_some()
        }));
    }
}

use cdr_runtime::component_worker::handle_pending_text_reply;
use cdr_store::queue;
use serde_json::json;
use std::time::Duration;
#[path = "support/approval_app_server.rs"]
mod app;

#[tokio::test]
async fn secret_input_stays_pending_and_is_never_forwarded_from_discord() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = app::start(&temp, &log).await;
    server.request("test/pending", json!({"method":"item/tool/requestUserInput","questions":[{
        "id":"q","header":"Private","question":"Private input","isSecret":true,"options":null
    }]}), Duration::from_secs(2), None).await.unwrap();
    let before = server.pending_server_requests(None).await.unwrap();
    let result =
        handle_pending_text_reply("thread-b", "public-safe fixture", &server, &db, 42, 3).await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("secret input requires the Codex app")
    );
    assert_eq!(before, server.pending_server_requests(None).await.unwrap());
    server.close().await.unwrap();
    assert!(!std::fs::read_to_string(log).unwrap().lines().any(|line| {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        value["id"] == "approval-1" && value.get("result").is_some()
    }));
}

#[tokio::test]
async fn text_answers_share_original_actor_room_and_turn_checks() {
    for method in [
        "item/commandExecution/requestApproval",
        "item/tool/requestUserInput",
    ] {
        for (channel, user, ended, allowed) in [
            (42, 4, false, false),
            (43, 3, false, false),
            (42, 3, true, false),
            (42, 3, false, true),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let db = temp.path().join("db.sqlite");
            let log = temp.path().join("rpc.jsonl");
            let server = app::start(&temp, &log).await;
            queue::enqueue(
                &db,
                queue::NewQueueJob {
                    job_id: "owner",
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
            if ended {
                server
                    .request("test/finish", json!({}), Duration::from_secs(2), None)
                    .await
                    .unwrap();
            }
            let result =
                handle_pending_text_reply("thread-b", "1", &server, &db, channel, user).await;
            let pending = server.pending_server_requests(None).await.unwrap();
            server.close().await.unwrap();
            if allowed {
                assert!(result.unwrap().is_some());
                assert!(pending.is_empty());
            } else {
                assert!(result.is_err());
                assert_eq!(pending.len(), 1);
            }
            let responses = std::fs::read_to_string(log)
                .unwrap()
                .lines()
                .filter(|line| {
                    let value: serde_json::Value = serde_json::from_str(line).unwrap();
                    value["id"] == "approval-1" && value.get("result").is_some()
                })
                .count();
            assert_eq!(responses, usize::from(allowed));
        }
    }
}

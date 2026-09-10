use cdr_runtime::{
    server_prompt_delivery::{self, PromptDeliveryContext},
    server_prompt_redisplay,
};
use cdr_store::queue;
use std::{sync::Arc, time::Duration};
#[path = "support/approval_app_server.rs"]
mod app;
#[path = "../src/completion_worker/goal_mirror_http.rs"]
mod http_gate;

#[tokio::test]
async fn long_input_frame_reaches_client_registry() {
    let temp = tempfile::tempdir().unwrap();
    let client =
        cdr_app_server::AppServerClient::start(app::config(&temp, &temp.path().join("rpc.jsonl")))
            .await
            .unwrap();
    client.request("test/pending", serde_json::json!({"requestId":"long-input", "method":"item/tool/requestUserInput", "questions":[{"id":"q","question":"긴 질문".repeat(1200),"options":[{"label":"First"}]}]}), Duration::from_secs(2)).await.unwrap();
    let pending = client.pending_server_requests(None);
    let diagnostic = client.diagnostic_snapshot();
    client.close().await.unwrap();
    assert_eq!(pending.len(), 1, "fixture diagnostic: {diagnostic:?}");
}

#[tokio::test]
async fn multiple_long_existing_prompts_keep_all_content_and_reuse_each_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let log = temp.path().join("rpc.jsonl");
    let server = Arc::new(app::start(&temp, &log).await);
    let generation = i64::try_from(server.generation()).unwrap();
    queue::enqueue(
        &db,
        queue::NewQueueJob {
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
    queue::begin_attempt(&db, "owner", &[], generation).unwrap();
    queue::mark_running(&db, "owner", "turn-b", generation).unwrap();
    for index in 0..6 {
        let mut params = serde_json::json!({"requestId":format!("pending-{index}")});
        if index == 5 {
            params["method"] = serde_json::json!("item/tool/requestUserInput");
            params["questions"] = serde_json::json!([{"id":"q","question":"긴 질문".repeat(1200),
                "options":[{"label":"First"},{"label":"Second"}]}]);
        }
        server
            .request("test/pending", params, Duration::from_secs(2), None)
            .await
            .unwrap();
    }
    let before = server.pending_server_requests(None).await.unwrap();
    let prepared = server_prompt_redisplay::prepare(&db, &server, "thread-b", 42, 3)
        .await
        .unwrap();
    assert_eq!(
        prepared.len(),
        6,
        "pending fixture IDs: {:?}",
        before
            .iter()
            .map(|r| (&r.id, &r.method))
            .collect::<Vec<_>>()
    );
    let expected: usize = prepared
        .iter()
        .map(|p| cdr_discord::text::split_delivery_chunks(&p.prompt.text, true).len())
        .sum();
    assert!(expected > 6);
    let gate = http_gate::start().await;
    let http = twilight_http::Client::builder()
        .proxy(gate.address, true)
        .ratelimiter(None)
        .build();
    gate.release.send(()).unwrap();
    let context = PromptDeliveryContext {
        database: &db,
        server: &server,
        http: &http,
        channel_id: 42,
        user_id: 3,
        command_key: "message:102",
    };
    for _ in 0..2 {
        server_prompt_delivery::deliver(&context, &prepared)
            .await
            .unwrap();
    }
    gate.entered.await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(
        posts.len(),
        expected,
        "same command retry must not resend pages"
    );
    assert_eq!(
        posts
            .iter()
            .filter(|p| p["components"]
                .as_array()
                .is_some_and(|rows| !rows.is_empty()))
            .count(),
        6
    );
    assert!(
        posts
            .iter()
            .all(|p| p["content"].as_str().unwrap().encode_utf16().count() <= 2000)
    );
    assert_eq!(server.pending_server_requests(None).await.unwrap(), before);
    server.close().await.unwrap();
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(!calls.contains("thread/resume") && !calls.contains("thread/fork"));
}

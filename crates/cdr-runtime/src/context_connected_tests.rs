use crate::test_support::mapped_slash as slash;
use crate::test_support::{approval_http, message_fixture::MessageFixture};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn prefix_and_slash_context_refresh_deliver_real_recent_text() {
    for mode in ["prefix", "query", "refresh"] {
        let is_slash = mode != "prefix";
        let root = tempfile::tempdir().unwrap();
        let http = approval_http::start().await;
        let client = Arc::new(
            twilight_http::Client::builder()
                .proxy(http.address, true)
                .ratelimiter(None)
                .build(),
        );
        let fixture = MessageFixture::new(&root, client.clone()).await;
        let path = root.path().join("context.jsonl");
        std::fs::write(&path,concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":12345}}}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"phase\":\"final\",\"content\":[{\"type\":\"output_text\",\"text\":\"마지막 완료 답변\"}]}}\n"
        )).unwrap();
        rusqlite::Connection::open(root.path().join("state.sqlite"))
            .unwrap()
            .execute(
                "UPDATE threads SET rollout_path=? WHERE id='thread-b'",
                [path.to_string_lossy().as_ref()],
            )
            .unwrap();
        let server = fixture.server.clone();
        if is_slash {
            let work = if mode == "query" {
                slash::stage(fixture.executor.mirror_db(), "context").await
            } else {
                slash::stage_options(
                    fixture.executor.mirror_db(),
                    "context",
                    serde_json::json!([{ "name":"refresh","type":5,"value":true }]),
                )
                .await
            };
            let (send, recv) = tokio::sync::mpsc::channel(1);
            send.send(work).await.unwrap();
            drop(send);
            tokio::time::timeout(
                Duration::from_secs(5),
                crate::interaction_worker::run_interaction_worker(
                    recv,
                    Arc::new(fixture.executor),
                    server.clone(),
                    client,
                ),
            )
            .await
            .unwrap();
        } else {
            crate::message_worker::process_admitted_gateway_message(
                fixture.admit("!context refresh 5"),
                &fixture.context(root.path()),
            )
            .await
            .unwrap();
        }
        http.stop.send(()).unwrap();
        let traffic = http.task.await.unwrap();
        assert_eq!(traffic.len(), 1);
        assert_eq!(traffic[0].0, !is_slash);
        let text = traffic[0].1["content"].as_str().unwrap();
        assert!(text.contains("last_input: 12345"));
        assert_eq!(
            text.contains("assistant final") && text.contains("마지막 완료 답변"),
            mode != "query"
        );
        assert!(!text.contains("recent_events:"));
        assert!(!root.path().join("bridge.json").exists());
        server.close().await.unwrap();
        let calls = std::fs::read_to_string(root.path().join("rpc.jsonl")).unwrap();
        assert!(!calls.contains("thread/") && !calls.contains("turn/"));
    }
}

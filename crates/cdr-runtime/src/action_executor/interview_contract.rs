use crate::test_support::approval_http as http_fixture;
use crate::test_support::message_fixture::MessageFixture;
use std::{sync::Arc, time::Duration};
#[path = "interview_slash_fixture.rs"]
mod mapped_slash;

#[tokio::test]
async fn prefix_and_slash_deliver_the_complete_python_interview_contract() {
    for slash in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let gate = http_fixture::start().await;
        let http = Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        let fixture = MessageFixture::new(&temp, http.clone()).await;
        let server = fixture.server.clone();
        let db = fixture.executor.mirror_db().to_owned();
        if slash {
            let work = mapped_slash::stage(&db, "interview").await;
            let (send, recv) = tokio::sync::mpsc::channel(1);
            send.send(work).await.unwrap();
            drop(send);
            tokio::time::timeout(
                Duration::from_secs(3),
                crate::interaction_worker::run_interaction_worker(
                    recv,
                    Arc::new(fixture.executor),
                    server.clone(),
                    http,
                ),
            )
            .await
            .unwrap();
        } else {
            crate::message_worker::process_admitted_gateway_message(
                fixture.admit("!interview 원래 요청"),
                &fixture.context(temp.path()),
            )
            .await
            .unwrap();
        }
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        server.close().await.unwrap();
        let frames = crate::test_support::app_fixture::rpc_log(&temp.path().join("rpc.jsonl"));
        let starts: Vec<_> = frames
            .iter()
            .filter(|frame| frame["method"] == "turn/start")
            .collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["params"]["threadId"], "thread-b");
        let sent = starts[0]["params"]["input"][0]["text"].as_str().unwrap();
        let python =
            include_str!("../../../../codex_discord_prefix_skill_prompts.py").replace("\r\n", "\n");
        let expected = python
            .split("DEEP_INTERVIEW_PROMPT_HEADER = \"\"\"")
            .nth(1)
            .unwrap()
            .split("\"\"\"")
            .next()
            .unwrap();
        assert_eq!(
            sent,
            format!("{expected}원래 요청"),
            "full procedure must reach actual turn/start"
        );
        assert_eq!(
            posts.len(),
            1,
            "one initial reply, not an extra header echo"
        );
        assert_eq!(posts[0].0, !slash, "prefix POST / original slash PATCH");
        assert_eq!(posts[0].1["content"], "In progress\nmessage: 원래 요청");
        let stored = cdr_store::queue::list(&db).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].target_thread_id, "thread-b");
        assert_eq!(
            stored[0].prompt, sent,
            "durable prompt is not the display preview"
        );
        assert!(!frames.iter().any(|frame| frame["method"] == "thread/fork"));
    }
}

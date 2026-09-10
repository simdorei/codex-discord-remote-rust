use crate::test_support::mapped_slash as slash;
use crate::test_support::{approval_http, message_fixture::MessageFixture};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn display_commands_reach_prefix_and_original_slash_reply_without_mutation() {
    for (command, expected) in [
        ("list", "used"),
        ("archived_list", "archived_at:"),
        ("where", "cwd: C:/repos/beta"),
        ("status", "goal 조회 실패"),
    ] {
        for is_slash in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let http = approval_http::start().await;
            let client = Arc::new(
                twilight_http::Client::builder()
                    .proxy(http.address, true)
                    .ratelimiter(None)
                    .build(),
            );
            let fixture = MessageFixture::new(&root, client.clone()).await;
            let server = fixture.server.clone();
            if is_slash {
                let work = slash::stage(fixture.executor.mirror_db(), command).await;
                let (send, recv) = tokio::sync::mpsc::channel(1);
                send.send(work).await.unwrap();
                drop(send);
                tokio::time::timeout(
                    Duration::from_secs(8),
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
                    fixture.admit(&format!("!{command}")),
                    &fixture.context(root.path()),
                )
                .await
                .unwrap();
            }
            http.stop.send(()).unwrap();
            let traffic = http.task.await.unwrap();
            assert!(!traffic.is_empty());
            assert_eq!(traffic[0].0, !is_slash);
            let text = traffic
                .iter()
                .map(|(_, value)| value["content"].as_str().unwrap())
                .collect::<String>();
            assert!(text.contains(expected), "{command}/{is_slash}: {text}");
            assert!(!root.path().join("bridge.json").exists());
            server.close().await.unwrap();
            let frames = crate::test_support::app_fixture::rpc_log(&root.path().join("rpc.jsonl"));
            assert!(frames.iter().all(|frame| matches!(
                frame["method"].as_str(),
                Some("initialize" | "thread/read" | "thread/goal/get")
            )));
            if matches!(command, "where" | "archived_list") {
                assert_eq!(frames.len(), 1);
            }
        }
    }
}

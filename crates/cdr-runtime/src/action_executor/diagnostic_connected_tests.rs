use crate::test_support::{approval_http, mapped_slash, message_fixture::MessageFixture};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn diagnostic_commands_use_real_prefix_and_slash_delivery_without_server_mutation() {
    let _guard = crate::resource_report::TEST_HOST_PROBE.lock().await;
    // Resources is prefix-only in both Python and the registered Rust catalog.
    for (command, slash) in [("doctor", false), ("doctor", true), ("resources", false)] {
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
        if slash {
            let work = mapped_slash::stage(fixture.executor.mirror_db(), command).await;
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
        assert_eq!(traffic[0].0, !slash);
        let text = traffic
            .iter()
            .map(|(_, value)| value["content"].as_str().unwrap())
            .collect::<String>();
        if command == "doctor" {
            assert!(
                text.contains("state_db: read OK") && text.contains("bridge_state: 조회 실패"),
                "{text}"
            );
        } else if cfg!(windows) {
            assert!(
                text.contains("CPU:") && text.contains("RAM:") && text.contains("Disk:"),
                "{text}"
            );
        } else {
            assert!(text.contains("Windows 실측 API만 지원"));
        }
        assert!(!root.path().join("bridge.json").exists());
        server.close().await.unwrap();
        let frames = crate::test_support::app_fixture::rpc_log(&root.path().join("rpc.jsonl"));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["method"], "initialize");
    }
}

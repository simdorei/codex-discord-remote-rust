use super::*;
use crate::test_support::{approval_app_fixture as app, message_fixture::MessageFixture};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn explicit_pro_never_answers_pending_input_even_during_drain() {
    for raw in ["!pro 확인", "!pro review 검수"] {
        for drain in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let log = temp.path().join("rpc.jsonl");
            let server = Arc::new(app::start(&temp, &log).await);
            let gate = crate::test_support::http_gate::start().await;
            let fixture = MessageFixture::with_server(
                &temp,
                Arc::new(
                    Client::builder()
                        .proxy(gate.address, true)
                        .ratelimiter(None)
                        .build(),
                ),
                server.clone(),
            );
            crate::test_support::approval_owner::running(
                fixture.executor.mirror_db(),
                server.generation(),
            );
            server
                .request(
                    "test/pending",
                    json!({"method":"item/tool/requestUserInput",
                "questions":[{"id":"q","question":"Describe change","options":null}]}),
                    Duration::from_secs(2),
                    None,
                )
                .await
                .unwrap();
            let before = server.pending_server_requests(None).await.unwrap();
            let admitted = fixture.admit(raw);
            let admitted = if drain {
                admitted.require_pending_reply().unwrap()
            } else {
                admitted
            };
            gate.release.send(()).unwrap();
            let result =
                process_admitted_gateway_message(admitted, &fixture.context(temp.path())).await;
            assert_eq!(
                before,
                server.pending_server_requests(None).await.unwrap(),
                "explicit Pro command was consumed as a question answer: {raw}"
            );
            if drain {
                assert!(matches!(result, Err(MessageWorkerError::Restarting)));
            } else {
                assert!(
                    result.is_ok(),
                    "explicit Pro must reach the busy choice: {result:?}"
                );
            }
            gate.stop.send(()).unwrap();
            let _ = gate.task.await.unwrap();
            server.close().await.unwrap();
            assert!(!std::fs::read_to_string(log).unwrap().lines().any(|line| {
                let frame: serde_json::Value = serde_json::from_str(line).unwrap();
                (frame["id"] == "approval-1" && frame.get("result").is_some())
                    || matches!(
                        frame["method"].as_str(),
                        Some("turn/start" | "turn/steer" | "thread/start" | "thread/fork")
                    )
            }));
        }
    }
}

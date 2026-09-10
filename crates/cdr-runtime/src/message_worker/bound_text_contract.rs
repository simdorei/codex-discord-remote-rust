use super::*;
use crate::test_support::{approval_app_fixture as app, message_fixture::MessageFixture};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn displayed_text_binding_answers_only_one_of_two_pending_inputs() {
    for options in [
        serde_json::Value::Null,
        json!([
            {"label":"First"},{"label":"Second"},{"label":"Third"},
            {"label":"Fourth"},{"label":"Fifth"},{"label":"Sixth"}
        ]),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("rpc.jsonl");
        let server = Arc::new(app::start(&temp, &log).await);
        let gate = crate::test_support::http_gate::start().await;
        let http = Arc::new(
            Client::builder()
                .proxy(gate.address, true)
                .ratelimiter(None)
                .build(),
        );
        let fixture = MessageFixture::with_server(&temp, http, server.clone());
        let db = fixture.executor.mirror_db();
        crate::test_support::approval_owner::running(db, server.generation());
        for id in ["input-one", "input-two"] {
            server.request("test/pending",json!({"requestId":id,"method":"item/tool/requestUserInput","questions":[{"id":"q","question":"Choose","options":options}]}),Duration::from_secs(2),None).await.unwrap();
        }
        gate.release.send(()).unwrap();
        process_admitted_gateway_message(fixture.admit("!approval"), &fixture.context(temp.path()))
            .await
            .unwrap();
        let prepared = crate::server_prompt_redisplay::prepare(db, &server, "thread-b", 42, 3)
            .await
            .unwrap();
        let first = prepared
            .iter()
            .find(|p| p.request.id == cdr_app_server::RequestId::String("input-one".into()))
            .unwrap();
        let binding = first
            .prompt
            .text
            .lines()
            .find_map(|line| {
                line.strip_prefix("[codex-reply:")
                    .and_then(|s| s.split_once(']'))
                    .map(|(token, _)| format!("[codex-reply:{token}]"))
            })
            .expect("display must provide an executable exact-request text binding");
        assert!(
            process_admitted_gateway_message(
                fixture.admit_id("6", 802),
                &fixture.context(temp.path())
            )
            .await
            .is_err()
        );
        process_admitted_gateway_message(
            fixture.admit_id(&format!("{binding} 6"), 803),
            &fixture.context(temp.path()),
        )
        .await
        .unwrap();
        assert!(
            process_admitted_gateway_message(
                fixture.admit_id(&format!("{binding} 6"), 804),
                &fixture.context(temp.path())
            )
            .await
            .is_err()
        );
        let pending = server.pending_server_requests(None).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].id,
            cdr_app_server::RequestId::String("input-two".into())
        );
        server.request("test/pending",json!({"requestId":"input-one","method":"item/tool/requestUserInput","questions":[{"id":"q","question":"Replacement","options":null}]}),Duration::from_secs(2),None).await.unwrap();
        assert!(
            process_admitted_gateway_message(
                fixture.admit_id(&format!("{binding} 6"), 805),
                &fixture.context(temp.path())
            )
            .await
            .is_err()
        );
        assert_eq!(server.pending_server_requests(None).await.unwrap().len(), 2);
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        assert!(
            posts
                .iter()
                .any(|post| post["content"].as_str().unwrap().contains(&binding))
        );
        server.close().await.unwrap();
        assert_exact_reply(&log, options.is_null());
    }
}

#[tokio::test]
async fn actual_message_worker_never_submits_malformed_input() {
    for questions in [
        json!([{"id":"q","question":"Choose","options":[{"label":""},{"label":"Second"}]}]),
        json!([{"id":"q","question":"First","options":null},{"id":" q ","question":"Second","options":null}]),
    ] {
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
        gate.release.send(()).unwrap();
        crate::test_support::approval_owner::running(
            fixture.executor.mirror_db(),
            server.generation(),
        );
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
        let result =
            process_admitted_gateway_message(fixture.admit("1"), &fixture.context(temp.path()))
                .await;
        assert!(result.is_err());
        assert_eq!(before, server.pending_server_requests(None).await.unwrap());
        gate.stop.send(()).unwrap();
        assert!(gate.task.await.unwrap().is_empty());
        server.close().await.unwrap();
        assert!(!std::fs::read_to_string(log).unwrap().lines().any(|line| {
            let frame: serde_json::Value = serde_json::from_str(line).unwrap();
            (frame["id"] == "approval-1" && frame.get("result").is_some())
                || matches!(
                    frame["method"].as_str(),
                    Some("turn/start" | "thread/start" | "thread/fork")
                )
        }));
    }
}

fn assert_exact_reply(log: &Path, free_text: bool) {
    let frames: Vec<serde_json::Value> = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let replies: Vec<_> = frames
        .iter()
        .filter(|frame| frame["id"] == "input-one" && frame.get("result").is_some())
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(
        replies[0]["result"]["answers"]["q"]["answers"],
        json!([if free_text { "6" } else { "Sixth" }])
    );
    assert!(
        !frames
            .iter()
            .any(|frame| frame["id"] == "input-two" && frame.get("result").is_some())
    );
    assert!(!frames.iter().any(|frame| matches!(
        frame["method"].as_str(),
        Some("thread/fork" | "turn/start" | "thread/start")
    )));
}

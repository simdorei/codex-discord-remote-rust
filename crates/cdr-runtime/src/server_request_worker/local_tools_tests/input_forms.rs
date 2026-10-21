use super::{Fixture, request};
use crate::component_worker::handle_pending_text_reply;
use crate::config::{ApprovalPolicy, LocalToolApprovalPolicy};
use serde_json::{Value, json};
use std::sync::Arc;

#[tokio::test]
async fn mcp_input_forms_stay_pending_then_submit_validated_typed_answers() {
    for mode in [Some("form"), Some("openai/form"), None] {
        for policy in [ApprovalPolicy::Ask, ApprovalPolicy::Allow] {
            run_case(mode, policy).await;
        }
    }
}

async fn run_case(mode: Option<&str>, policy: ApprovalPolicy) {
    let mut input = request();
    input.params["serverName"] = json!("fixture-mcp");
    input.params["message"] = json!("Choose the destination and copies");
    if let Some(mode) = mode {
        input.params["mode"] = json!(mode);
    } else {
        input.params.as_object_mut().unwrap().remove("mode");
    }
    input.params["requestedSchema"] = json!({"type":"object","properties":{
        "folder":{"type":"string","minLength":2},"copies":{"type":"integer","minimum":1}
    },"required":["folder","copies"],"additionalProperties":false});
    let mut fixture = Fixture::with_request(Some(3), input).await;
    fixture.worker.approval_policy = policy;
    fixture.worker.local_tool_approval_policy = LocalToolApprovalPolicy::Allow;
    let generation = fixture.worker.server.generation();
    let gate = crate::test_support::http_gate::start().await;
    fixture.worker.http = Arc::new(
        twilight_http::Client::builder()
            .proxy(gate.address.clone(), true)
            .ratelimiter(None)
            .build(),
    );
    gate.release.send(()).unwrap();
    let result = fixture
        .worker
        .process(generation, fixture.request.clone())
        .await;
    let pending = fixture
        .worker
        .server
        .pending_server_requests(None)
        .await
        .unwrap();
    if result.is_err() || pending.is_empty() {
        gate.stop.send(()).unwrap();
        gate.task.await.unwrap();
        fixture.worker.server.close().await.unwrap();
        panic!(
            "F1: input must be delivered and stay pending: {result:?}, count={}",
            pending.len()
        );
    }
    fixture
        .worker
        .process(generation, fixture.request.clone())
        .await
        .unwrap();
    fixture.worker.sent = super::SentRequestCache::default();
    fixture.worker.recover().await.unwrap();
    gate.stop.send(()).unwrap();
    let posts = gate.task.await.unwrap();
    assert_eq!(posts.len(), 1, "recovery must reuse the prompt receipt");
    let content = posts[0]["content"].as_str().unwrap();
    for expected in [
        "fixture-mcp",
        "destination",
        "folder",
        "copies",
        "required",
        "/cancel",
        "/decline",
    ] {
        assert!(content.contains(expected), "missing {expected}");
    }
    submit_and_verify(&fixture, generation).await;
}

async fn submit_and_verify(fixture: &Fixture, generation: u64) {
    let token = crate::server_prompt::text_binding::fingerprint(&fixture.request, generation);
    let prefix = format!("[codex-reply:{token}] ");
    let valid = format!("{prefix}{{\"folder\":\"C:/work\",\"copies\":2}}");
    let db = &fixture.worker.mirror_db;
    let server = &fixture.worker.server;
    assert!(
        handle_pending_text_reply("thread-b", &valid, server, db, 42, 4)
            .await
            .is_err()
    );
    let stale = crate::server_prompt::text_binding::fingerprint(&fixture.request, generation + 1);
    assert!(
        handle_pending_text_reply(
            "thread-b",
            &format!("[codex-reply:{stale}] {{}}"),
            server,
            db,
            42,
            3
        )
        .await
        .is_err()
    );
    let invalid = format!("{prefix}{{\"folder\":\"C:/work\",\"copies\":0}}");
    assert!(
        handle_pending_text_reply("thread-b", &invalid, server, db, 42, 3)
            .await
            .is_err()
    );
    assert_eq!(server.pending_server_requests(None).await.unwrap().len(), 1);
    assert!(
        handle_pending_text_reply("thread-b", &valid, server, db, 42, 3)
            .await
            .unwrap()
            .unwrap()
            .contains("submitted")
    );
    assert!(
        server
            .pending_server_requests(None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        handle_pending_text_reply("thread-b", &valid, server, db, 42, 3)
            .await
            .is_err()
    );
    server.close().await.unwrap();
    let responses: Vec<Value> = std::fs::read_to_string(&fixture.log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|v| v["id"] == "approval-1" && v.get("result").is_some())
        .collect();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["result"],
        json!({"action":"accept","content":{"folder":"C:/work","copies":2},"_meta":null})
    );
}

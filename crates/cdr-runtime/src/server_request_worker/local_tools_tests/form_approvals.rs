use super::{Fixture, local_tools, request};
use crate::config::ApprovalPolicy;
use crate::server_request_worker::approval_policy;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn form_approvals_annotations_do_not_change_an_empty_approval() {
    for key in ["title", "description", "$schema", "$id", "$comment"] {
        let mut approval = request();
        approval.params["serverName"] = json!("codex_apps");
        approval.params["_meta"]["connector_id"] = json!("github");
        approval.params["requestedSchema"][key] = json!("annotation");
        assert_eq!(
            approval_policy::response(&approval, ApprovalPolicy::Allow).unwrap(),
            Some(json!({"action":"accept","content":{},"_meta":null})),
            "{key}"
        );
        assert!(
            local_tools::response(&approval).is_none(),
            "ask must leave {key} pending"
        );
        approval.params["requestedSchema"][key] = json!(42);
        assert!(
            approval_policy::response(&approval, ApprovalPolicy::Allow)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn form_approvals_ask_delivers_external_prompt_and_submits_original_choice() {
    for mode in ["form", "openai/form"] {
        let mut approval = request();
        approval.params["mode"] = json!(mode);
        approval.params["serverName"] = json!("codex_apps");
        approval.params["_meta"] = json!({"connector_id":"github","riskLevel":"high"});
        approval.params["message"] = json!("Allow GitHub to publish this commit?");
        approval.params["requestedSchema"]["title"] = json!("Approval");
        let mut fixture = Fixture::with_request(Some(3), approval).await;
        let generation = fixture.worker.server.generation();
        let gate = crate::test_support::http_gate::start().await;
        fixture.worker.http = Arc::new(
            twilight_http::Client::builder()
                .proxy(gate.address.clone(), true)
                .ratelimiter(None)
                .build(),
        );
        gate.release.send(()).unwrap();
        fixture
            .worker
            .process(generation, fixture.request.clone())
            .await
            .unwrap();
        let pending = fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap();
        // Clean up the fixture even when the pre-fix implementation cancels.
        if pending.is_empty() {
            gate.stop.send(()).unwrap();
            gate.task.await.unwrap();
            fixture.worker.server.close().await.unwrap();
            panic!("ask must keep the original external approval pending");
        }
        fixture
            .worker
            .process(generation, fixture.request.clone())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), gate.entered)
            .await
            .unwrap()
            .unwrap();
        gate.stop.send(()).unwrap();
        let posts = gate.task.await.unwrap();
        assert_eq!(posts.len(), 1);
        assert!(
            posts[0]["content"]
                .as_str()
                .unwrap()
                .contains("Allow GitHub")
        );
        let button = posts[0]["components"][0]["components"][0]["custom_id"]
            .as_str()
            .unwrap();
        let component = cdr_discord::components::parse_component_id(button).unwrap();
        let authority = crate::server_prompt_authority::verify(
            &fixture.worker.mirror_db,
            &fixture.worker.server,
            &pending[0],
            generation,
        )
        .await
        .unwrap();
        assert!(authority.require_actor(42, 4).is_err());
        authority.require_actor(42, 3).unwrap();
        let response =
            crate::component_worker::build_component_response(&component, &pending, generation)
                .unwrap();
        assert!(
            crate::component_worker::build_component_response(&component, &pending, generation + 1)
                .is_err()
        );
        assert_eq!(response.occurrence, fixture.request.occurrence);
        assert_eq!(
            response.payload,
            json!({"action":"accept","content":{},"_meta":null})
        );
        fixture
            .worker
            .server
            .respond_current(
                &response.request_id,
                response.occurrence,
                response.payload,
                response.generation,
            )
            .await
            .unwrap();
        assert_answered_once(&fixture).await;
    }
}

async fn assert_answered_once(fixture: &Fixture) {
    assert!(
        fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.worker.server.close().await.unwrap();
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    let replies: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|v| v["id"] == "approval-1" && v.get("result").is_some())
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(
        replies[0]["result"],
        json!({"action":"accept","content":{},"_meta":null})
    );
}

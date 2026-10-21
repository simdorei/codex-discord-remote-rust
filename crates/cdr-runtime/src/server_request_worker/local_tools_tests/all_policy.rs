use super::{Fixture, request};
use crate::config::{ApprovalPolicy, CliOptions, RuntimeConfig};
use crate::server_request_worker::approval_policy;
use cdr_app_server::ServerRequest;
use serde_json::{Value, json};
use std::time::Duration;

fn cases() -> Vec<(ServerRequest, Value)> {
    let mut cases = Vec::new();
    for (method, response) in [
        (
            "item/commandExecution/requestApproval",
            json!({"decision":"accept"}),
        ),
        (
            "item/fileChange/requestApproval",
            json!({"decision":"accept"}),
        ),
        ("execCommandApproval", json!({"decision":"approved"})),
        ("applyPatchApproval", json!({"decision":"approved"})),
        (
            "item/permissions/requestApproval",
            json!({"permissions":{"network":{"enabled":true},"fileSystem":{"write":["C:/fixture"]}},"scope":"turn"}),
        ),
    ] {
        let mut approval = request();
        approval.method = method.into();
        approval.params = json!({"threadId":"thread-b","turnId":"turn-b","permissions":{"network":{"enabled":true},"fileSystem":{"write":["C:/fixture"]}}});
        cases.push((approval, response));
    }
    for mode in ["form", "openai/form", "url"] {
        let mut approval = request();
        approval.params["serverName"] = json!("codex_apps");
        approval.params["mode"] = json!(mode);
        approval.params["url"] = json!("https://example.com/approval");
        approval.params["_meta"] = json!({"codex_approval_kind":"mcp_tool_call","connector_id":"github","riskLevel":"high","tool_name":"github_create_tree"});
        cases.push((
            approval,
            json!({"action":"accept","content":{},"_meta":null}),
        ));
    }
    // The all-approval setting must not require a connector or risk allowlist.
    let mut approval = request();
    approval.params["serverName"] = json!("another-mcp");
    approval.params.as_object_mut().unwrap().remove("_meta");
    cases.push((
        approval,
        json!({"action":"accept","content":{},"_meta":null}),
    ));
    let mut annotated = request();
    annotated.params["serverName"] = json!("codex_apps");
    annotated.params["requestedSchema"] = json!({"type":"object","properties":{},
        "title":"Approval","description":"Approve this action", "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":"https://example.com/approval", "$comment":"No input required"});
    cases.push((
        annotated,
        json!({"action":"accept","content":{},"_meta":null}),
    ));
    cases
}

#[test]
fn all_approval_policy_uses_each_protocols_accept_response() {
    for (request, expected) in cases() {
        assert_eq!(
            approval_policy::response(&request, ApprovalPolicy::Allow).unwrap(),
            Some(expected),
            "{}",
            request.method
        );
        assert!(
            approval_policy::response(&request, ApprovalPolicy::Ask)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn all_approval_policy_does_not_invent_input_values() {
    let mut request = request();
    request.params["requestedSchema"] =
        json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]});
    assert!(
        approval_policy::response(&request, ApprovalPolicy::Allow)
            .unwrap()
            .is_none()
    );
    for method in [
        "item/tool/requestUserInput",
        "item/tool/call",
        "future/request",
    ] {
        request.method = method.into();
        assert!(
            approval_policy::response(&request, ApprovalPolicy::Allow)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn all_approval_policy_configuration_is_explicit_and_validated() {
    let mut env = [
        ("DISCORD_BOT_TOKEN".into(), "fixture".into()),
        ("DISCORD_ALLOW_ALL_CHANNELS".into(), "1".into()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        RuntimeConfig::from_map(&env, CliOptions::default())
            .unwrap()
            .approval_policy,
        ApprovalPolicy::Ask
    );
    for (raw, expected) in [
        ("", ApprovalPolicy::Ask),
        ("ask", ApprovalPolicy::Ask),
        ("allow", ApprovalPolicy::Allow),
        (" allow ", ApprovalPolicy::Allow),
    ] {
        env.insert("DISCORD_APPROVAL_POLICY".into(), raw.into());
        assert_eq!(
            RuntimeConfig::from_map(&env, CliOptions::default())
                .unwrap()
                .approval_policy,
            expected
        );
    }
    env.insert("DISCORD_APPROVAL_POLICY".into(), "ALLOW".into());
    assert!(RuntimeConfig::from_map(&env, CliOptions::default()).is_err());
}

#[tokio::test]
async fn all_approval_policy_answers_actual_resident_without_a_prompt() {
    for (approval, expected) in cases() {
        let mut fixture = Fixture::with_request(Some(3), approval).await;
        fixture.worker.approval_policy = ApprovalPolicy::Allow;
        let generation = fixture.worker.server.generation();
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            fixture.worker.process(generation, fixture.request.clone()),
        )
        .await;
        let pending = fixture
            .worker
            .server
            .pending_server_requests(None)
            .await
            .unwrap();
        if matches!(outcome, Ok(Ok(()))) {
            assert!(
                fixture
                    .worker
                    .process(generation, fixture.request.clone())
                    .await
                    .is_err(),
                "a completed approval must not be replayed"
            );
        }
        fixture.worker.server.close().await.unwrap();
        assert!(
            matches!(outcome, Ok(Ok(()))),
            "all approval policy must answer without HTTP: {outcome:?}"
        );
        assert!(pending.is_empty());
        assert_eq!(
            fixture.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        let responses: Vec<Value> = log
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|v| v["id"] == "approval-1" && v.get("result").is_some())
            .collect();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["result"], expected);
        assert!(!log.contains("config/read"));
    }
}

#[tokio::test]
async fn all_approval_policy_keeps_original_authority_checks() {
    for owner in [None, Some(3)] {
        let mut approval = request();
        approval.params["serverName"] = json!("codex_apps");
        let mut fixture = Fixture::with_request(owner, approval).await;
        fixture.worker.approval_policy = ApprovalPolicy::Allow;
        let generation = fixture.worker.server.generation();
        if owner.is_some() {
            fixture
                .worker
                .server
                .request("test/finish", json!({}), Duration::from_secs(2), None)
                .await
                .unwrap();
        }
        assert!(
            fixture
                .worker
                .process(generation, fixture.request.clone())
                .await
                .is_err()
        );
        fixture.worker.server.close().await.unwrap();
        let log = std::fs::read_to_string(&fixture.log).unwrap();
        assert!(
            !log.lines()
                .map(|s| serde_json::from_str::<Value>(s).unwrap())
                .any(|v| v["id"] == "approval-1" && v.get("result").is_some())
        );
        assert_eq!(
            fixture.listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

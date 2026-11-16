use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::codex_tools::{ToolReport, ToolState, inspect, inspect_with_managed_official};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

#[test]
#[ignore = "owned app-server protocol fixture"]
fn tools_protocol_fixture() {
    let mode = std::env::var("CDR_TOOLS_CASE").unwrap();
    let root = std::path::PathBuf::from(std::env::var_os("CDR_TOOLS_ROOT").unwrap());
    let mut output = std::io::stdout().lock();
    let mut inventories = 0;
    let mut calls = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls.jsonl"))
        .unwrap();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: Value = serde_json::from_str(&line).unwrap();
        let Some(id) = message.get("id") else {
            continue;
        };
        let method = message["method"].as_str().unwrap();
        let result = match method {
            "initialize" => json!({}),
            "thread/start" => {
                assert_eq!(message["params"]["ephemeral"], true);
                assert_eq!(message["params"]["sandbox"], "read-only");
                assert_eq!(message["params"]["approvalPolicy"], "never");
                json!({"thread":{"id":"fixture-thread","ephemeral":true}})
            }
            "mcpServerStatus/list" => {
                inventories += 1;
                let mut node = json!({"name":"node_repl","tools":{"js":{"inputSchema":{"type":"object"}}},"authStatus":"unsupported"});
                if mode == "changed" && inventories > 1 {
                    node["tools"]["js"]["inputSchema"] = json!({"changed":true});
                }
                let mut data = vec![node];
                if mode != "missing" {
                    let mut cua = json!({"name":"cua_repl","pluginId":"unified-computer-use@openai-bundled","tools":{"js":{}},"authStatus":if mode=="auth" {"unknown"}else{"unsupported"}});
                    if mode == "missing-owner" {
                        cua.as_object_mut().unwrap().remove("pluginId");
                    }
                    if mode == "wrong-owner" {
                        cua["pluginId"] = json!("different-plugin@openai-bundled");
                    }
                    if mode == "alternate-name" {
                        cua["name"] = json!("owned-cua-server");
                    }
                    if mode == "discovery-error" {
                        cua["toolsError"] =
                            json!("Cannot verify browser access without the current turn");
                    }
                    data.push(cua);
                }
                json!({"data":data,"nextCursor":null})
            }
            "mcpServer/tool/call" => {
                assert_eq!(message["params"]["threadId"], "fixture-thread");
                assert_eq!(message["params"]["server"], "node_repl");
                assert_eq!(message["params"]["tool"], "js");
                writeln!(calls, "{}", message["params"]).unwrap();
                let code = message["params"]["arguments"]["code"].as_str().unwrap();
                assert!(code.contains("node-js-v1"));
                assert!(!code.contains("cua."));
                if mode == "failure" {
                    json!({"isError":true,"content":[{"type":"text","text":"spawn EPERM: fixture failure"}]})
                } else {
                    json!({"isError":false,"content":[{"type":"text","text":json!({"cdrProbe":"node-js-v1","observed":true}).to_string()}]})
                }
            }
            other => panic!("unexpected method {other}"),
        };
        let response = if mode == "protocol-error" && method == "mcpServer/tool/call" {
            json!({"id":id,"error":{"code":-32000,"message":"spawn EPERM: protocol fixture"}})
        } else {
            json!({"id":id,"result":result})
        };
        writeln!(output, "{response}").unwrap();
        output.flush().unwrap();
    }
}

async fn exercise(mode: &str, invoke: bool, managed: bool) -> (ToolReport, usize) {
    let root = tempfile::tempdir().unwrap();
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "tools_protocol_fixture".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config
        .environment
        .insert("CDR_TOOLS_CASE".into(), mode.into());
    config
        .environment
        .insert("CDR_TOOLS_ROOT".into(), root.path().display().to_string());
    let client = AppServerClient::start(config).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let report = if managed {
        inspect_with_managed_official(&client, root.path(), invoke, true, deadline).await
    } else {
        inspect(&client, root.path(), invoke, deadline).await
    }
    .unwrap();
    let serialized = serde_json::to_value(&report).unwrap();
    assert_eq!(
        serialized["verification_scope"],
        "javascript_and_mcp_inventory"
    );
    assert_eq!(serialized["native_desktop_state"], "NOT_CHECKED");
    assert_eq!(serialized["browser_state"], "NOT_CHECKED");
    assert_eq!(serialized["desktop_app_independence"], "NOT_CHECKED");
    client.close().await.unwrap();
    assert!(report.catalog_sha256.is_some());
    let calls = std::fs::read_to_string(root.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .count();
    (report, calls)
}

#[tokio::test]
async fn plain_javascript_success_never_verifies_computer_use() {
    let (catalog, calls) = exercise("normal", false, false).await;
    assert_eq!(catalog.state, ToolState::NotChecked);
    assert_eq!(calls, 0);
    let (probed, calls) = exercise("normal", true, false).await;
    assert_eq!(calls, 1);
    assert_eq!(probed.state, ToolState::NotChecked);
    assert_eq!(probed.checks[0].state, ToolState::Verified);
    assert_eq!(probed.checks[1].state, ToolState::NotChecked);
    assert_eq!(probed.checks[1].reason, "requires_active_model_turn");
}

#[tokio::test]
async fn missing_auth_failure_and_changed_catalog_never_verify() {
    assert_eq!(
        exercise("missing", true, false).await.0.state,
        ToolState::Failed
    );
    assert_eq!(
        exercise("auth", true, false).await.0.state,
        ToolState::Unverified
    );
    assert_eq!(
        exercise("failure", true, false).await.0.state,
        ToolState::Failed
    );
    assert_eq!(
        exercise("changed", true, false).await.0.state,
        ToolState::Unverified
    );
}

#[tokio::test]
async fn plain_cua_name_is_accepted_only_for_an_explicit_managed_registration() {
    assert_eq!(
        exercise("missing-owner", true, false).await.0.state,
        ToolState::Failed
    );
    let (managed, calls) = exercise("missing-owner", true, true).await;
    assert_eq!(managed.state, ToolState::NotChecked);
    assert_eq!(calls, 1);
    assert_eq!(managed.checks[1].server.as_deref(), Some("cua_repl"));
    assert_eq!(
        exercise("wrong-owner", true, true).await.0.state,
        ToolState::Failed
    );
    assert_eq!(
        exercise("alternate-name", true, false).await.0.state,
        ToolState::NotChecked
    );
}

#[tokio::test]
async fn tool_protocol_and_discovery_errors_preserve_the_original_reason() {
    for mode in ["failure", "protocol-error"] {
        let (report, calls) = exercise(mode, true, false).await;
        assert_eq!(calls, 1);
        assert!(
            report.checks[0]
                .error
                .as_deref()
                .unwrap()
                .contains("spawn EPERM")
        );
    }
    let (report, calls) = exercise("discovery-error", true, false).await;
    assert_eq!(calls, 1);
    assert_eq!(
        report.checks[1].error.as_deref(),
        Some("Cannot verify browser access without the current turn")
    );
}

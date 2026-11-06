use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::codex_tools::{ToolState, inspect};
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
        let result = match message["method"].as_str().unwrap() {
            "initialize" => json!({}),
            "thread/start" => {
                assert_eq!(message["params"]["ephemeral"], true);
                assert_eq!(message["params"]["sandbox"], "read-only");
                json!({"thread":{"id":"fixture-thread"}})
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
                    data.push(cua);
                }
                json!({"data":data,"nextCursor":null})
            }
            "mcpServer/tool/call" => {
                assert_eq!(message["params"]["threadId"], "fixture-thread");
                assert_eq!(message["params"]["tool"], "js");
                writeln!(calls, "{}", message["params"]).unwrap();
                let code = message["params"]["arguments"]["code"]
                    .as_str()
                    .unwrap_or("");
                let marker = if code.contains("node-js-v1") {
                    "node-js-v1"
                } else {
                    "cua-state-v1"
                };
                json!({"isError":mode=="failure","content":[{"type":"text","text":json!({"cdrProbe":marker,"observed":true}).to_string()}]})
            }
            other => panic!("unexpected method {other}"),
        };
        writeln!(output, "{}", json!({"id":id,"result":result})).unwrap();
        output.flush().unwrap();
    }
}

async fn exercise(mode: &str, invoke: bool) -> (ToolState, usize) {
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
    let report = inspect(
        &client,
        root.path(),
        invoke,
        Instant::now() + Duration::from_secs(10),
    )
    .await
    .unwrap();
    let serialized = serde_json::to_value(&report).unwrap();
    assert_eq!(
        serialized["verification_scope"],
        "javascript_and_surface_inventory"
    );
    assert_eq!(serialized["native_desktop_state"], "NOT_CHECKED");
    assert_eq!(serialized["desktop_app_independence"], "NOT_CHECKED");
    client.close().await.unwrap();
    assert!(report.catalog_sha256.is_some());
    let calls = std::fs::read_to_string(root.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .count();
    (report.state, calls)
}

#[tokio::test]
async fn only_actual_calls_can_verify_the_fixed_contract() {
    assert_eq!(exercise("normal", false).await, (ToolState::NotChecked, 0));
    assert_eq!(exercise("normal", true).await, (ToolState::Verified, 4));
}
#[tokio::test]
async fn missing_auth_failure_and_changed_catalog_never_verify() {
    assert_eq!(exercise("missing", true).await.0, ToolState::Failed);
    assert_eq!(exercise("auth", true).await.0, ToolState::Unverified);
    assert_ne!(exercise("failure", true).await.0, ToolState::Verified);
    assert_eq!(exercise("changed", true).await.0, ToolState::Unverified);
}

#[tokio::test]
async fn successful_markers_cannot_replace_required_plugin_ownership() {
    assert_eq!(
        exercise("missing-owner", true).await,
        (ToolState::Failed, 1)
    );
    assert_eq!(exercise("wrong-owner", true).await, (ToolState::Failed, 1));
    assert_eq!(
        exercise("alternate-name", true).await,
        (ToolState::Verified, 4)
    );
}

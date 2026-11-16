use cdr_app_server::{AppServerClient, AppServerConfig};
use cdr_runtime::local_computer::probe::inspect_with_managed_official;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

#[test]
#[ignore = "owned inventory-only app-server fixture"]
fn official_computer_inventory_fixture() {
    let mode = std::env::var("CDR_OFFICIAL_INVENTORY_CASE").unwrap();
    let path = std::env::var_os("CDR_OFFICIAL_INVENTORY_CALLS").unwrap();
    let mut calls = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    let mut output = std::io::stdout().lock();
    let mut pages = 0;
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let message: Value = serde_json::from_str(&line).unwrap();
        let Some(id) = message.get("id") else {
            continue;
        };
        let method = message["method"].as_str().unwrap();
        writeln!(calls, "{method}").unwrap();
        let result = match method {
            "initialize" => json!({}),
            "thread/start" => {
                assert_eq!(message["params"]["ephemeral"], true);
                assert_eq!(message["params"]["sandbox"], "read-only");
                assert_eq!(message["params"]["approvalPolicy"], "never");
                json!({"thread":{"id":"inventory-thread","ephemeral":mode!="persistent"}})
            }
            "mcpServerStatus/list" => {
                assert_eq!(message["params"]["threadId"], "inventory-thread");
                pages += 1;
                let mut server = json!({
                    "name":"cua_repl",
                    "pluginId":"unified-computer-use@openai-bundled",
                    "authStatus":"unsupported",
                    "tools":{"js":{},"js_reset":{}}
                });
                match mode.as_str() {
                    "plain" => {
                        server.as_object_mut().unwrap().remove("pluginId");
                    }
                    "wrong-owner" => {
                        server["pluginId"] = json!("other@openai-bundled");
                    }
                    "legacy" => {
                        server["name"] = json!("cdr_computer");
                        server.as_object_mut().unwrap().remove("pluginId");
                    }
                    "malformed-js" => {
                        server["tools"]["js"] = Value::Null;
                    }
                    "discovery-error" => {
                        server["toolsError"] = json!("spawn EPERM: fixture discovery");
                    }
                    _ => {}
                }
                let next = if mode == "repeat-cursor" || (mode == "duplicate-page" && pages == 1) {
                    json!("next")
                } else {
                    Value::Null
                };
                let data = if mode == "repeat-cursor" && pages > 1 {
                    vec![]
                } else {
                    vec![server]
                };
                json!({"data":data,"nextCursor":next})
            }
            other => panic!("inventory-only probe must not call {other}"),
        };
        writeln!(output, "{}", json!({"id":id,"result":result})).unwrap();
        output.flush().unwrap();
    }
}

async fn exercise(mode: &str, managed: bool) -> Result<Value, String> {
    let root = tempfile::tempdir().unwrap();
    let calls = root.path().join("calls.txt");
    let mut config = AppServerConfig::new(std::env::current_exe().unwrap());
    config.arguments = vec![
        "--exact".into(),
        "official_computer_inventory_fixture".into(),
        "--ignored".into(),
        "--nocapture".into(),
    ];
    config
        .environment
        .insert("CDR_OFFICIAL_INVENTORY_CASE".into(), mode.into());
    config.environment.insert(
        "CDR_OFFICIAL_INVENTORY_CALLS".into(),
        calls.display().to_string(),
    );
    let client = AppServerClient::start(config).await.unwrap();
    let report = inspect_with_managed_official(
        &client,
        root.path(),
        managed,
        Instant::now() + Duration::from_secs(10),
    )
    .await
    .map_err(|error| error.to_string());
    client.close().await.unwrap();
    let methods = std::fs::read_to_string(calls).unwrap();
    assert!(methods.lines().all(|method| matches!(
        method,
        "initialize" | "thread/start" | "mcpServerStatus/list"
    )));
    report
}

#[tokio::test]
async fn official_inventory_never_invokes_cua_or_claims_live_turn_success() {
    for (mode, managed) in [("normal", false), ("plain", true)] {
        let report = exercise(mode, managed).await.unwrap();
        assert_eq!(report["inventory_state"], "VERIFIED");
        assert_eq!(report["verification_scope"], "official_mcp_inventory_only");
        for field in [
            "state",
            "native_desktop_state",
            "browser_state",
            "desktop_app_independence",
        ] {
            assert_eq!(report[field], "NOT_CHECKED", "{field}");
        }
        assert!(report.get("window_count").is_none());
        if managed {
            assert!(report["plugin_id"].is_null());
        }
    }
}

#[tokio::test]
async fn wrong_identity_partial_inventory_and_persistent_threads_fail_closed() {
    for (mode, managed) in [
        ("plain", false),
        ("wrong-owner", true),
        ("legacy", true),
        ("malformed-js", true),
        ("repeat-cursor", false),
        ("duplicate-page", false),
        ("persistent", false),
    ] {
        assert!(exercise(mode, managed).await.is_err(), "{mode}");
    }
    let error = exercise("discovery-error", false).await.unwrap_err();
    assert!(error.contains("spawn EPERM: fixture discovery"));
}

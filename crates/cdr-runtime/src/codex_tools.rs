//! Explicit read-only probes, separate from CLI selection and normal bot startup.
use cdr_app_server::{AppServerClient, AppServerError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

pub const CONTRACT_ID: &str = "local-tools-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolState {
    NotChecked,
    Unverified,
    Failed,
    Verified,
}

#[derive(Debug, Serialize)]
pub struct ToolCheck {
    pub requirement: &'static str,
    pub server: Option<String>,
    pub tool: &'static str,
    pub state: ToolState,
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ToolReport {
    pub contract_id: &'static str,
    pub verification_scope: &'static str,
    pub state: ToolState,
    pub native_desktop_state: ToolState,
    pub desktop_app_independence: ToolState,
    pub checks: Vec<ToolCheck>,
    pub catalog_sha256: Option<String>,
}

fn catalog_checks(catalog: &[Value]) -> Vec<ToolCheck> {
    ["node_repl", "cua_repl"]
        .into_iter()
        .map(|required| {
            let matches: Vec<_> = catalog
                .iter()
                .filter(|server| {
                    (required == "node_repl" && server["name"].as_str() == Some(required))
                        || (required == "cua_repl"
                            && server["pluginId"].as_str().is_some_and(|id| {
                                id.split('@').next() == Some("unified-computer-use")
                            }))
                })
                .collect();
            let mut check = ToolCheck {
                requirement: required,
                server: None,
                tool: "js",
                state: ToolState::Failed,
                reason: "required_server_missing",
            };
            if matches.len() > 1 {
                check.state = ToolState::Unverified;
                check.reason = "ambiguous_server";
            }
            if let [server] = matches.as_slice() {
                check.server = server["name"].as_str().map(str::to_owned);
                if !matches!(
                    server["authStatus"].as_str(),
                    Some("unsupported" | "bearerToken" | "oAuth")
                ) || !server["toolsError"].is_null()
                {
                    check.state = ToolState::Unverified;
                    check.reason = "auth_or_discovery_unverified";
                } else if server["tools"].get("js").is_some() {
                    check.state = ToolState::NotChecked;
                    check.reason = "invocation_not_checked";
                } else {
                    check.reason = "required_tool_missing";
                }
            }
            check
        })
        .collect()
}

#[must_use]
pub fn classify_catalog(catalog: &[Value]) -> ToolReport {
    report(catalog_checks(catalog))
}

fn report(checks: Vec<ToolCheck>) -> ToolReport {
    let state = [
        ToolState::Failed,
        ToolState::Unverified,
        ToolState::NotChecked,
    ]
    .into_iter()
    .find(|state| checks.iter().any(|check| check.state == *state))
    .unwrap_or(ToolState::Verified);
    ToolReport {
        contract_id: CONTRACT_ID,
        verification_scope: "javascript_and_surface_inventory",
        state,
        native_desktop_state: ToolState::NotChecked,
        desktop_app_independence: ToolState::NotChecked,
        checks,
        catalog_sha256: None,
    }
}

async fn request(
    client: &AppServerClient,
    method: &str,
    params: Value,
    deadline: Instant,
) -> Result<Value, AppServerError> {
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(30));
    if remaining.is_zero() {
        return Err(AppServerError::StartupHeld(
            "tool diagnostic deadline".into(),
        ));
    }
    client.request(method, params, remaining).await
}

async fn inventory(
    client: &AppServerClient,
    thread: &str,
    deadline: Instant,
) -> Result<Vec<Value>, AppServerError> {
    let mut output = vec![];
    let mut cursor = Value::Null;
    for _ in 0..8 {
        let page = request(
            client,
            "mcpServerStatus/list",
            json!({"threadId":thread,"limit":100,"cursor":cursor,"detail":"toolsAndAuthOnly"}),
            deadline,
        )
        .await?;
        output.extend(
            page["data"]
                .as_array()
                .ok_or_else(|| AppServerError::InvalidReply {
                    message: "MCP inventory data missing".into(),
                })?
                .iter()
                .cloned(),
        );
        cursor = page["nextCursor"].clone();
        if cursor.is_null() {
            return Ok(output);
        }
    }
    Err(AppServerError::InvalidReply {
        message: "MCP inventory pagination incomplete".into(),
    })
}

fn valid_marker(result: &Value, marker: &str) -> bool {
    result["isError"] != true
        && result["content"].as_array().is_some_and(|content| {
            content.iter().any(|item| {
                item["text"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .is_some_and(|value| value["cdrProbe"] == marker && value["observed"] == true)
            })
        })
}

async fn call(
    client: &AppServerClient,
    thread: &str,
    server: &str,
    arguments: Value,
    deadline: Instant,
) -> Result<Value, AppServerError> {
    request(
        client,
        "mcpServer/tool/call",
        json!({"threadId":thread,"server":server,"tool":"js","arguments":arguments}),
        deadline,
    )
    .await
}

pub async fn inspect(
    client: &AppServerClient,
    cwd: &Path,
    invoke: bool,
    deadline: Instant,
) -> Result<ToolReport, AppServerError> {
    let started = request(
        client,
        "thread/start",
        json!({"cwd":cwd,"ephemeral":true,"approvalPolicy":"never","sandbox":"read-only"}),
        deadline,
    )
    .await?;
    let thread = started["thread"]["id"]
        .as_str()
        .ok_or_else(|| AppServerError::InvalidReply {
            message: "diagnostic thread identity missing".into(),
        })?;
    let before = inventory(client, thread, deadline).await?;
    let mut checks = catalog_checks(&before);
    if invoke {
        for check in &mut checks {
            if check.state != ToolState::NotChecked {
                continue;
            }
            let server = check.server.as_deref().expect("observed server");
            let outcome = if check.requirement == "node_repl" {
                call(client, thread, server, json!({"code":"nodeRepl.write(JSON.stringify({cdrProbe:'node-js-v1',observed:6*7===42}));","title":"Checking local JavaScript"}), deadline).await
                    .map(|result| valid_marker(&result, "node-js-v1"))
            } else {
                let observed = call(client, thread, server, json!({"code":"var cdrCuaState = await cua.getState();","title":"Checking surface inventory"}), deadline).await;
                let verified = match observed {
                    Ok(result) if result["isError"] != true => call(client, thread, server, json!({"code":"nodeRepl.write(JSON.stringify({cdrProbe:'cua-state-v1',observed:cdrCuaState!==undefined}));","title":"Verifying inventory observation"}), deadline).await
                        .map(|result| valid_marker(&result, "cua-state-v1")),
                    Ok(_) => Ok(false),
                    Err(error) => Err(error),
                };
                let completed = call(
                    client,
                    thread,
                    server,
                    json!({"completion_summary":"Checked surface inventory"}),
                    deadline,
                )
                .await;
                match completed {
                    Ok(result) if result["isError"] != true => verified,
                    _ => Err(AppServerError::StartupHeld(
                        "native probe completion unverified".into(),
                    )),
                }
            };
            match outcome {
                Ok(true) => {
                    check.state = ToolState::Verified;
                    check.reason = "read_only_probe_passed";
                }
                Ok(false) => {
                    check.state = ToolState::Failed;
                    check.reason = "probe_result_mismatch";
                }
                Err(_) => {
                    check.state = ToolState::Unverified;
                    check.reason = "probe_call_unverified";
                }
            }
        }
        let after = inventory(client, thread, deadline).await?;
        let before_identity = catalog_identity(&before);
        if before_identity != catalog_identity(&after) {
            for check in &mut checks {
                check.state = ToolState::Unverified;
                check.reason = "catalog_changed";
            }
        }
    }
    let mut report = report(checks);
    report.catalog_sha256 = Some(hex::encode(Sha256::digest(
        catalog_identity(&before).to_string().as_bytes(),
    )));
    Ok(report)
}

fn catalog_identity(catalog: &[Value]) -> Value {
    let mut rows: Vec<_> = catalog.iter().map(|server| json!({"name":server["name"],"pluginId":server["pluginId"],"authStatus":server["authStatus"],"toolsError":server["toolsError"],"tools":server["tools"],"runtimeStatus":server["runtimeStatus"],"serverInfo":server["serverInfo"]})).collect();
    rows.sort_by_key(Value::to_string);
    json!(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_required_tools_and_auth_are_never_ready() {
        assert_eq!(classify_catalog(&[]).state, ToolState::Failed);
        let node = json!({"name":"node_repl","tools":{"js":{}},"authStatus":"unsupported"});
        assert_eq!(
            classify_catalog(std::slice::from_ref(&node)).state,
            ToolState::Failed
        );
        let cua = json!({"name":"cua_repl","pluginId":"unified-computer-use@openai-bundled","tools":{"js":{}},"authStatus":"unsupported"});
        assert_eq!(
            classify_catalog(&[node.clone(), cua.clone()]).state,
            ToolState::NotChecked
        );
        let mut pending = cua.clone();
        pending["authStatus"] = json!("notLoggedIn");
        assert_eq!(
            classify_catalog(&[node.clone(), pending]).state,
            ToolState::Unverified
        );
        assert_eq!(
            classify_catalog(&[node, cua.clone(), cua]).state,
            ToolState::Unverified
        );
        assert!(!valid_marker(
            &json!({"isError":false,"content":[{"text":"tool started"}]}),
            "node-js-v1"
        ));
    }
}

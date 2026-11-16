//! Explicit read-only probes, separate from CLI selection and normal bot startup.
use cdr_app_server::{AppServerClient, AppServerError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, Instant};

pub const CONTRACT_ID: &str = "local-tools-v1";

const MAX_ERROR_BYTES: usize = 2_048;

/// Preserve the original diagnostic prefix without unbounded tool output.
pub(crate) fn bounded_error(message: &str) -> String {
    const SUFFIX: &str = " [truncated]";
    if message.len() <= MAX_ERROR_BYTES {
        return message.to_owned();
    }
    let mut end = MAX_ERROR_BYTES - SUFFIX.len();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{SUFFIX}", &message[..end])
}

pub(crate) fn bounded_value_error(value: &Value) -> String {
    match value {
        Value::String(message) => bounded_error(message),
        _ => bounded_error(&value.to_string()),
    }
}

/// A plain canonical name is trusted only after this runtime registered the
/// verified official launch source. Never infer plugin ownership from its name.
pub(crate) fn is_official_cua_server(server: &Value, managed_official: bool) -> bool {
    let plugin = server.get("pluginId");
    plugin
        .and_then(Value::as_str)
        .is_some_and(|id| id.split('@').next() == Some("unified-computer-use"))
        || (managed_official && server["name"] == "cua_repl" && plugin.is_none_or(Value::is_null))
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolReport {
    pub contract_id: &'static str,
    pub verification_scope: &'static str,
    pub state: ToolState,
    pub native_desktop_state: ToolState,
    pub browser_state: ToolState,
    pub desktop_app_independence: ToolState,
    pub checks: Vec<ToolCheck>,
    pub catalog_sha256: Option<String>,
}

fn catalog_checks(catalog: &[Value], managed_official: bool) -> Vec<ToolCheck> {
    ["node_repl", "cua_repl"]
        .into_iter()
        .map(|required| {
            let matches: Vec<_> = catalog
                .iter()
                .filter(|server| {
                    (required == "node_repl" && server["name"].as_str() == Some(required))
                        || (required == "cua_repl"
                            && is_official_cua_server(server, managed_official))
                })
                .collect();
            let mut check = ToolCheck {
                requirement: required,
                server: None,
                tool: "js",
                state: ToolState::Failed,
                reason: "required_server_missing",
                error: None,
            };
            if matches.len() > 1 {
                check.state = ToolState::Unverified;
                check.reason = "ambiguous_server";
            }
            if let [server] = matches.as_slice() {
                check.server = server["name"].as_str().map(str::to_owned);
                if !server["toolsError"].is_null() {
                    check.state = ToolState::Unverified;
                    check.reason = "auth_or_discovery_unverified";
                    check.error = Some(bounded_value_error(&server["toolsError"]));
                } else if !matches!(
                    server["authStatus"].as_str(),
                    Some("unsupported" | "bearerToken" | "oAuth")
                ) {
                    check.state = ToolState::Unverified;
                    check.reason = "auth_or_discovery_unverified";
                    check.error = Some(bounded_value_error(&server["authStatus"]));
                } else if server["tools"].get("js").is_some_and(Value::is_object) {
                    check.state = ToolState::NotChecked;
                    check.reason = if required == "cua_repl" {
                        "requires_active_model_turn"
                    } else {
                        "invocation_not_checked"
                    };
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
    classify_catalog_with_managed_official(catalog, false)
}

/// The caller must establish the managed registration before passing true.
#[must_use]
pub fn classify_catalog_with_managed_official(
    catalog: &[Value],
    managed_official: bool,
) -> ToolReport {
    report(catalog_checks(catalog, managed_official))
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
        verification_scope: "javascript_and_mcp_inventory",
        state,
        native_desktop_state: ToolState::NotChecked,
        browser_state: ToolState::NotChecked,
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
    inspect_with_managed_official(client, cwd, invoke, false, deadline).await
}

/// Inspect MCP registration and optionally exercise plain JavaScript. CUA
/// operations need a real model turn and are never invoked by this diagnostic.
pub async fn inspect_with_managed_official(
    client: &AppServerClient,
    cwd: &Path,
    invoke: bool,
    managed_official: bool,
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
    let mut checks = catalog_checks(&before, managed_official);
    if invoke {
        for check in &mut checks {
            if check.state != ToolState::NotChecked || check.requirement != "node_repl" {
                continue;
            }
            let server = check.server.as_deref().expect("observed server");
            let outcome = call(
                client,
                thread,
                server,
                json!({
                    "code":"nodeRepl.write(JSON.stringify({cdrProbe:'node-js-v1',observed:6*7===42}));",
                    "title":"Checking local JavaScript"
                }),
                deadline,
            )
            .await;
            match outcome {
                Ok(result) if valid_marker(&result, "node-js-v1") => {
                    check.state = ToolState::Verified;
                    check.reason = "read_only_probe_passed";
                }
                Ok(result) => {
                    check.state = ToolState::Failed;
                    check.reason = "probe_result_mismatch";
                    check.error = Some(bounded_value_error(&result));
                }
                Err(error) => {
                    check.state = ToolState::Unverified;
                    check.reason = "probe_call_unverified";
                    check.error = Some(bounded_error(&error.to_string()));
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
    #[test]
    fn plain_cua_requires_managed_registration_and_never_forges_plugin_ownership() {
        let node = json!({"name":"node_repl","tools":{"js":{}},"authStatus":"unsupported"});
        let plain = json!({"name":"cua_repl","tools":{"js":{}},"authStatus":"unsupported"});
        assert_eq!(
            classify_catalog(&[node.clone(), plain.clone()]).state,
            ToolState::Failed
        );
        let report = classify_catalog_with_managed_official(&[node.clone(), plain.clone()], true);
        assert_eq!(report.state, ToolState::NotChecked);
        assert_eq!(report.checks[1].reason, "requires_active_model_turn");
        assert!(plain.get("pluginId").is_none());

        let mut wrong_owner = plain.clone();
        wrong_owner["pluginId"] = json!("other-plugin@openai-bundled");
        assert_eq!(
            classify_catalog_with_managed_official(&[node, wrong_owner], true).state,
            ToolState::Failed
        );
        let mut legacy = plain;
        legacy["name"] = json!("cdr_computer");
        assert!(!is_official_cua_server(&legacy, true));
    }

    #[test]
    fn discovery_error_preserves_original_text_with_a_utf8_byte_bound() {
        let message = format!("spawn EPERM: {}", "한".repeat(3_000));
        let server = json!({
            "name":"cua_repl",
            "tools":{"js":{}},
            "authStatus":"unsupported",
            "toolsError":message
        });
        let report = classify_catalog_with_managed_official(&[server], true);
        let error = report.checks[1].error.as_deref().unwrap();
        assert!(error.starts_with("spawn EPERM: "));
        assert!(error.ends_with(" [truncated]"));
        assert!(error.len() <= MAX_ERROR_BYTES);
        assert_eq!(
            bounded_error("FrameArrived timed out"),
            "FrameArrived timed out"
        );
    }
}

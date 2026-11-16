//! Inventory-only diagnostic for the official Computer Use MCP.
//!
//! A thread-scoped MCP call does not supply a model turn's CUA context. Native
//! apps, browsers, and desktop-app independence require a separate live-turn test.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use cdr_app_server::{AppServerClient, AppServerError};
use serde_json::{Value, json};

use crate::codex_tools::{bounded_value_error, is_official_cua_server};

/// Inspect configured official plugin inventory without invoking a CUA tool.
pub async fn inspect(
    client: &AppServerClient,
    cwd: &Path,
    deadline: Instant,
) -> Result<Value, AppServerError> {
    inspect_with_managed_official(client, cwd, false, deadline).await
}

/// `managed_official` must come from a successful runtime-owned registration,
/// not from an observed server name or an operator-supplied plugin identifier.
pub async fn inspect_with_managed_official(
    client: &AppServerClient,
    cwd: &Path,
    managed_official: bool,
    deadline: Instant,
) -> Result<Value, AppServerError> {
    let started = request(
        client,
        "thread/start",
        json!({"cwd":cwd,"ephemeral":true,"approvalPolicy":"never","sandbox":"read-only"}),
        deadline,
    )
    .await?;
    if started["thread"]["ephemeral"] != true {
        return Err(invalid(
            "Official computer inventory requires an ephemeral thread.",
        ));
    }
    let thread = started["thread"]["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .ok_or_else(|| invalid("Official computer inventory thread identity missing."))?;
    verify_inventory(client, thread, managed_official, deadline).await
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
            "Official computer inventory deadline exceeded.".into(),
        ));
    }
    client.request(method, params, remaining).await
}

async fn verify_inventory(
    client: &AppServerClient,
    thread: &str,
    managed_official: bool,
    deadline: Instant,
) -> Result<Value, AppServerError> {
    let mut cursor = Value::Null;
    let mut seen_cursors = BTreeSet::new();
    let mut found = None;
    for _ in 0..8 {
        let page = request(
            client,
            "mcpServerStatus/list",
            json!({"threadId":thread,"limit":100,"cursor":cursor,"detail":"toolsAndAuthOnly"}),
            deadline,
        )
        .await?;
        let servers = page["data"]
            .as_array()
            .ok_or_else(|| invalid("Official computer MCP inventory is malformed."))?;
        for server in servers
            .iter()
            .filter(|server| is_official_cua_server(server, managed_official))
        {
            if found.is_some() {
                return Err(invalid("Official computer MCP provider is ambiguous."));
            }
            found = Some(inventory_report(server)?);
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => {
                return found.ok_or_else(|| invalid("Official computer MCP provider is missing."));
            }
            Some(Value::String(next)) if !next.is_empty() && seen_cursors.insert(next.clone()) => {
                cursor = json!(next);
            }
            _ => {
                return Err(invalid(
                    "Official computer MCP cursor is malformed or repeated.",
                ));
            }
        }
    }
    Err(invalid(
        "Official computer MCP inventory pagination is incomplete.",
    ))
}

fn inventory_report(server: &Value) -> Result<Value, AppServerError> {
    let name = server["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("Official computer MCP provider name is malformed."))?;
    if let Some(error) = server.get("toolsError").filter(|error| !error.is_null()) {
        return Err(invalid(&format!(
            "Official computer MCP tool discovery failed: {}",
            bounded_value_error(error)
        )));
    }
    if !matches!(
        server["authStatus"].as_str(),
        Some("unsupported" | "bearerToken" | "oAuth")
    ) {
        return Err(invalid(&format!(
            "Official computer MCP authentication is unverified: {}",
            bounded_value_error(&server["authStatus"])
        )));
    }
    let tools = server["tools"]
        .as_object()
        .ok_or_else(|| invalid("Official computer MCP tool catalog is malformed."))?;
    if !tools.get("js").is_some_and(Value::is_object) {
        return Err(invalid(
            "Official computer MCP js tool is missing or malformed.",
        ));
    }
    Ok(json!({
        "contract_id": "official-computer-inventory-v1",
        "state": "NOT_CHECKED",
        "inventory_state": "VERIFIED",
        "provider": name,
        "plugin_id": server["pluginId"],
        "tool_count": tools.len(),
        "verification_scope": "official_mcp_inventory_only",
        "native_desktop_state": "NOT_CHECKED",
        "browser_state": "NOT_CHECKED",
        "desktop_app_independence": "NOT_CHECKED",
        "reason": "requires_active_model_turn",
    }))
}

fn invalid(message: &str) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn official() -> Value {
        json!({
            "name":"cua_repl",
            "pluginId":"unified-computer-use@openai-bundled",
            "authStatus":"unsupported",
            "tools":{"js":{},"js_reset":{}}
        })
    }

    #[test]
    fn a_complete_catalog_cannot_verify_cua_or_desktop_app_independence() {
        let report = inventory_report(&official()).unwrap();
        assert_eq!(report["inventory_state"], "VERIFIED");
        for field in [
            "state",
            "native_desktop_state",
            "browser_state",
            "desktop_app_independence",
        ] {
            assert_eq!(report[field], "NOT_CHECKED", "{field}");
        }
        assert_eq!(report["reason"], "requires_active_model_turn");
        assert!(report.get("window_count").is_none());
    }

    #[test]
    fn discovery_errors_preserve_the_bounded_original_reason() {
        let mut server = official();
        let message = format!("spawn EPERM: {}", "한".repeat(3_000));
        server["toolsError"] = json!(message);
        let error = inventory_report(&server).unwrap_err().to_string();
        assert!(error.contains("spawn EPERM"));
        assert!(error.contains("[truncated]"));
        assert!(error.len() < 2_200);
    }

    #[test]
    fn malformed_or_unready_catalogs_never_pass_inventory() {
        for (field, value) in [
            ("tools", json!({})),
            ("tools", json!({"js":null})),
            ("tools", json!([])),
            ("authStatus", json!("notLoggedIn")),
            ("name", json!("")),
        ] {
            let mut server = official();
            server[field] = value;
            assert!(inventory_report(&server).is_err(), "{field}");
        }
    }
}

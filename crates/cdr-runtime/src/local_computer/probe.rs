//! Read-only acceptance probe for the computer provider bundled into Remote.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use cdr_app_server::{AppServerClient, AppServerError};
use cdr_core::Validate;
use cdr_remote_protocol::output::ComputerOutput;
use serde_json::{Value, json};

const PROVIDER: &str = "cdr_computer";
const EXPECTED_TOOLS: [&str; 11] = [
    "computer_activate_window",
    "computer_click",
    "computer_close_window",
    "computer_drag",
    "computer_launch_app",
    "computer_list_windows",
    "computer_press_keys",
    "computer_screenshot",
    "computer_scroll",
    "computer_stop",
    "computer_type_text",
];

/// Verify the bundled MCP catalog and one native window-list operation.
/// No model turn or desktop input is requested, and window details stay private.
pub async fn inspect(
    client: &AppServerClient,
    cwd: &Path,
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
            "Bundled computer probe requires an ephemeral thread.",
        ));
    }
    let thread = started["thread"]["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 200)
        .ok_or_else(|| invalid("Bundled computer probe thread identity missing."))?;
    verify_inventory(client, thread, deadline).await?;
    let result = request(
        client,
        "mcpServer/tool/call",
        json!({"threadId":thread,"server":PROVIDER,"tool":"computer_list_windows","arguments":{}}),
        deadline,
    )
    .await?;
    let window_count = native_window_count(&result)?;
    Ok(json!({
        "state":"VERIFIED",
        "provider":PROVIDER,
        "tool_count":EXPECTED_TOOLS.len(),
        "window_count":window_count,
        "scope":"bundled_mcp_inventory_and_native_window_list",
    }))
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
            "Bundled computer probe deadline exceeded.".into(),
        ));
    }
    client.request(method, params, remaining).await
}

async fn verify_inventory(
    client: &AppServerClient,
    thread: &str,
    deadline: Instant,
) -> Result<(), AppServerError> {
    let mut cursor = Value::Null;
    let mut found = false;
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
            .ok_or_else(|| invalid("Bundled computer MCP inventory is malformed."))?;
        for server in servers.iter().filter(|server| server["name"] == PROVIDER) {
            if found {
                return Err(invalid("Bundled computer MCP provider is ambiguous."));
            }
            verify_catalog(server)?;
            found = true;
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => {
                return if found {
                    Ok(())
                } else {
                    Err(invalid("Bundled computer MCP provider is missing."))
                };
            }
            Some(Value::String(next)) if !next.is_empty() => cursor = json!(next),
            _ => return Err(invalid("Bundled computer MCP cursor is malformed.")),
        }
    }
    Err(invalid(
        "Bundled computer MCP inventory pagination is incomplete.",
    ))
}

fn verify_catalog(server: &Value) -> Result<(), AppServerError> {
    if server
        .get("toolsError")
        .is_some_and(|error| !error.is_null())
    {
        return Err(invalid("Bundled computer MCP tool discovery failed."));
    }
    let tools = server["tools"]
        .as_object()
        .filter(|tools| tools.len() == EXPECTED_TOOLS.len())
        .ok_or_else(|| invalid("Bundled computer MCP tool catalog is incomplete."))?;
    let mut names = BTreeSet::new();
    for (key, definition) in tools {
        if !definition.is_object() {
            return Err(invalid(
                "Bundled computer MCP tool definition is malformed.",
            ));
        }
        let name = definition
            .get("name")
            .map_or(Some(key.as_str()), Value::as_str)
            .ok_or_else(|| invalid("Bundled computer MCP tool name is malformed."))?;
        let name = name.strip_prefix("mcp__cdr_computer__").unwrap_or(name);
        if !EXPECTED_TOOLS.contains(&name) || !names.insert(name) {
            return Err(invalid(
                "Bundled computer MCP tool catalog differs from expected tools.",
            ));
        }
    }
    Ok(())
}

fn native_window_count(result: &Value) -> Result<usize, AppServerError> {
    if !result.is_object() {
        return Err(invalid("Bundled computer native result is malformed."));
    }
    match result.get("isError") {
        None | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => {
            return Err(invalid("Bundled computer native window listing failed."));
        }
        _ => return Err(invalid("Bundled computer native error flag is malformed.")),
    }
    let output: ComputerOutput = if let Some(structured) = result
        .get("structuredContent")
        .filter(|value| !value.is_null())
    {
        serde_json::from_value(structured.clone())
            .map_err(|_| invalid("Bundled computer native metadata is malformed."))?
    } else {
        let content = result["content"]
            .as_array()
            .ok_or_else(|| invalid("Bundled computer native content is missing."))?;
        let mut texts = content.iter().filter(|item| item["type"] == "text");
        let text = texts
            .next()
            .and_then(|item| item["text"].as_str())
            .ok_or_else(|| invalid("Bundled computer native text metadata is missing."))?;
        if texts.next().is_some() {
            return Err(invalid(
                "Bundled computer native text metadata is ambiguous.",
            ));
        }
        serde_json::from_str(text)
            .map_err(|_| invalid("Bundled computer native text metadata is malformed."))?
    };
    output
        .validate()
        .map_err(|_| invalid("Bundled computer native output failed validation."))?;
    match output {
        ComputerOutput::ComputerWindows { windows } => Ok(windows.len()),
        _ => Err(invalid(
            "Bundled computer native result is not a window list.",
        )),
    }
}

fn invalid(message: &str) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_list() -> Value {
        json!({"kind":"computer_windows","windows":[{
            "window_id":1,"title":"private title","process_name":"chrome.exe",
            "left":0,"top":0,"width":800,"height":600,"active":true,
        }]})
    }

    #[test]
    fn native_metadata_accepts_both_supported_result_encodings() {
        let output = window_list();
        assert_eq!(
            native_window_count(&json!({"isError":false,"structuredContent":output})).unwrap(),
            1
        );
        assert_eq!(
            native_window_count(&json!({"content":[{"type":"text","text":output.to_string()}]}))
                .unwrap(),
            1
        );
    }

    #[test]
    fn errors_malformed_windows_and_wrong_variants_never_verify() {
        let mut invalid_window = window_list();
        invalid_window["windows"][0]["width"] = json!(0);
        let text = json!({"type":"text","text":window_list().to_string()});
        for result in [
            json!({"isError":true,"structuredContent":window_list()}),
            json!({"isError":"false","structuredContent":window_list()}),
            json!({"structuredContent":invalid_window}),
            json!({"structuredContent":{"kind":"computer_action","action":"click","message":"sent"}}),
            json!({"structuredContent":{},"content":[text.clone()]}),
            json!({"content":[text.clone(),text]}),
            json!({"content":[{"type":"text","text":"not JSON"}]}),
            json!({"content":[]}),
        ] {
            assert!(native_window_count(&result).is_err());
        }
    }
}

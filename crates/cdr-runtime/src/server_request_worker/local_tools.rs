//! Automatic local-tool access for an already-authorized Discord request.
//!
//! The caller must verify the original request owner and use `respond_current`.
//! This does not grant command, filesystem, URL-login, or persistent permissions.

use cdr_app_server::ServerRequest;
use serde_json::{Value, json};

pub(super) fn response(request: &ServerRequest) -> Option<(Value, &'static str)> {
    let params = &request.params;
    if request.method != "mcpServer/elicitation/request"
        || !matches!(
            params.get("mode").and_then(Value::as_str),
            Some("form" | "openai/form")
        )
    {
        return None;
    }
    let allowed = params.get("serverName").and_then(Value::as_str) == Some("node_repl")
        && params
            .pointer("/_meta/codex_approval_kind")
            .and_then(Value::as_str)
            == Some("mcp_tool_call")
        && matches!(
            params
                .pointer("/_meta/connector_id")
                .and_then(Value::as_str),
            Some("computer-use" | "browser")
        )
        && params.pointer("/_meta/riskLevel").and_then(Value::as_str) == Some("low")
        && empty_approval_schema(params.get("requestedSchema"));
    if allowed {
        return Some((
            json!({"action": "accept", "content": {}, "_meta": null}),
            "local-tool-access-auto-accepted",
        ));
    }
    // Advertising form support must not leave an unrenderable form hanging.
    // Unknown tools, elevated-risk requests, and data-entry forms fail closed.
    Some((
        json!({"action": "cancel", "content": null, "_meta": null}),
        "unsupported-form-cancelled",
    ))
}

fn empty_approval_schema(schema: Option<&Value>) -> bool {
    let Some(schema) = schema.and_then(Value::as_object) else {
        return false;
    };
    schema.get("type").and_then(Value::as_str) == Some("object")
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
        && schema
            .get("required")
            .is_none_or(|value| value.as_array().is_some_and(Vec::is_empty))
        && schema
            .get("additionalProperties")
            .is_none_or(Value::is_boolean)
        && schema.keys().all(|key| {
            matches!(
                key.as_str(),
                "type" | "properties" | "required" | "additionalProperties"
            )
        })
}

//! Route local-tool access without swallowing requests that need user approval.
//!
//! The caller must verify the original request owner and use `respond_current`.
//! Saved app grants come only from the user's config/read layer. Native policy
//! must still offer persistence. Command, filesystem and URL requests are separate.

use cdr_app_server::ServerRequest;
use serde_json::{Value, json};

pub(super) fn policy_response(
    request: &ServerRequest,
    policy: crate::config::LocalToolApprovalPolicy,
) -> Option<(Value, &'static str)> {
    (policy == crate::config::LocalToolApprovalPolicy::Allow
        && is_local_tool_form(&request.method, &request.params)
        && matches!(
            request
                .params
                .pointer("/_meta/riskLevel")
                .and_then(Value::as_str),
            Some("low" | "high")
        ))
    .then(|| {
        (
            // The operator opted in to answering these requests. Do not fabricate
            // a persistent native grant or change native app/URL restrictions.
            json!({"action":"accept","content":{},"_meta":null}),
            "local-tool-policy-auto-accepted",
        )
    })
}

pub(super) fn saved_app_candidate(request: &ServerRequest) -> Option<&str> {
    let params = &request.params;
    if !is_local_tool_form(&request.method, params)
        || params.pointer("/_meta/connector_id")?.as_str()? != "computer-use"
        || !matches!(
            params.pointer("/_meta/riskLevel")?.as_str()?,
            "low" | "high"
        )
        || !params
            .pointer("/_meta/persist")?
            .as_array()?
            .iter()
            .any(|v| v.as_str() == Some("always"))
        || params.pointer("/_meta/codex_request_type").is_some()
        || params.pointer("/_meta/tool_name").is_some()
    {
        return None;
    }
    let tool_params = params.pointer("/_meta/tool_params")?.as_object()?;
    let app = tool_params.get("app")?.as_str()?;
    (tool_params.len() == 1 && !app.is_empty() && app.trim() == app && app != "computer-audio")
        .then_some(app)
}

pub(super) fn saved_access_response(
    request: &ServerRequest,
    config_read: &Value,
) -> Option<(Value, &'static str)> {
    let app = saved_app_candidate(request)?;
    let layers = config_read.get("layers")?.as_array()?;
    // Do not infer a grant from a project file, a disabled layer, or ambiguous
    // duplicate user layers. Read afresh for each request so revocation applies.
    if layers.iter().any(|layer| {
        layer.pointer("/config/computer_use/allow_persistent_approval") == Some(&Value::Bool(false))
    }) {
        return None;
    }
    let mut users = layers
        .iter()
        .filter(|layer| layer.pointer("/name/type").and_then(Value::as_str) == Some("user"));
    let user = users.next()?;
    if users.next().is_some()
        || user
            .get("disabledReason")
            .is_some_and(|value| !value.is_null())
    {
        return None;
    }
    let allowed = user
        .pointer("/config/computer_use/windows/always_allowed_app_ids")?
        .as_array()?;
    if !allowed.iter().all(|value| {
        value
            .as_str()
            .is_some_and(|id| !id.is_empty() && id.trim() == id)
    }) || !allowed.iter().any(|value| value.as_str() == Some(app))
    {
        return None;
    }
    Some((
        json!({"action":"accept","content":{},"_meta":{"persist":"always"}}),
        "saved-app-access-auto-accepted",
    ))
}

pub(super) fn response(request: &ServerRequest) -> Option<(Value, &'static str)> {
    let params = &request.params;
    if !is_form_request(&request.method, params) {
        return None;
    }
    if requires_user_approval(&request.method, params) {
        // Keep the request pending for the original user's bound approval button.
        return None;
    }
    if is_local_tool_form(&request.method, params)
        && params.pointer("/_meta/riskLevel").and_then(Value::as_str) == Some("low")
    {
        return Some((
            json!({"action": "accept", "content": {}, "_meta": null}),
            "local-tool-access-auto-accepted",
        ));
    }
    // A form that needs values or has unrecognized validation constraints cannot
    // be answered with an empty approval payload.
    Some((
        json!({"action": "cancel", "content": null, "_meta": null}),
        "unsupported-form-cancelled",
    ))
}

pub(crate) fn requires_user_approval(method: &str, params: &Value) -> bool {
    // Connector/risk allowlists decide automatic local access, not whether a
    // valid approval can be shown to its owner. Unknown tools must not be cancelled.
    is_form_request(method, params)
        && empty_approval_schema(params.get("requestedSchema"))
        && !(is_local_tool_form(method, params)
            && params.pointer("/_meta/riskLevel").and_then(Value::as_str) == Some("low"))
}

fn is_form_request(method: &str, params: &Value) -> bool {
    method == "mcpServer/elicitation/request"
        && matches!(
            params.get("mode").and_then(Value::as_str),
            Some("form" | "openai/form")
        )
}

fn is_local_tool_form(method: &str, params: &Value) -> bool {
    is_form_request(method, params)
        && params.get("serverName").and_then(Value::as_str) == Some("node_repl")
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
        && empty_approval_schema(params.get("requestedSchema"))
}

pub(super) fn empty_approval_schema(schema: Option<&Value>) -> bool {
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
        && schema.iter().all(|(key, value)| {
            match key.as_str() {
                "type" | "properties" | "required" | "additionalProperties" => true,
                // These descriptive fields introduce no user input or assertions.
                "title" | "description" | "$schema" | "$id" | "$comment" => value.is_string(),
                _ => false,
            }
        })
}

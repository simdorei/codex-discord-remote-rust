use super::{ServerPrompt, text_binding};
use cdr_app_server::ServerRequest;
use serde_json::Value;

pub(super) fn prompt(request: &ServerRequest, thread_id: String, generation: u64) -> ServerPrompt {
    let server = text(request.params.get("serverName"), "MCP server");
    let message = text(
        request.params.get("message"),
        "Please provide the requested information.",
    );
    let mut lines = vec![format!("MCP input required\nserver: {server}\n{message}")];
    if cdr_app_server::mcp_forms::validate_schema(&request.params).is_ok() {
        describe_fields(&request.params["requestedSchema"], &mut lines);
    } else {
        lines.push("This input form cannot be validated. The request remains pending. Complete it in the Codex app or ask the plugin to correct its schema.".into());
    }
    lines.push("Use /decline to refuse or /cancel to cancel this request.".into());
    lines.push(text_binding::example(request, generation));
    ServerPrompt {
        thread_id,
        text: lines.join("\n"),
        components: Vec::new(),
    }
}

fn describe_fields(schema: &Value, lines: &mut Vec<String>) {
    let properties = schema.get("properties").and_then(Value::as_object);
    let required = schema.get("required").and_then(Value::as_array);
    if let Some(properties) = properties {
        for (key, field) in properties {
            let requirement =
                if required.is_some_and(|keys| keys.iter().any(|v| v.as_str() == Some(key))) {
                    "required"
                } else {
                    "optional"
                };
            lines.push(format!("- {key} ({requirement}): {field}"));
        }
    }
    if properties.is_none_or(serde_json::Map::is_empty) {
        lines.push(format!("Requested fields: {schema}"));
    }
    lines.push(
        if properties.is_some_and(|fields| fields.len() == 1) {
            "Reply with the value for this field, or a JSON object."
        } else {
            "Reply with one JSON object using the field names above."
        }
        .into(),
    );
    lines.push("Example format: {\"field_name\":\"value\"}. Enter numbers and true/false without quotes. Omit optional fields you do not want to send.".into());
}

fn text(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .chars()
        .take(1000)
        .collect()
}

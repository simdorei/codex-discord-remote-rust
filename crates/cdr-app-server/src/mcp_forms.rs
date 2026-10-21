//! MCP forms are input requests, not implicit approval or cancellation decisions.
use crate::AppServerError;
use serde_json::{Value, json};

mod input;
mod schema;

#[must_use]
pub fn is_form_request(method: &str, params: &Value) -> bool {
    method == "mcpServer/elicitation/request"
        && (matches!(
            params.get("mode").and_then(Value::as_str),
            Some("form" | "openai/form")
        ) || (params.get("mode").is_none() && params.get("requestedSchema").is_some()))
}

#[must_use]
pub fn needs_input(method: &str, params: &Value) -> bool {
    is_form_request(method, params) && !empty_approval_schema(params.get("requestedSchema"))
}

#[must_use]
pub fn empty_approval_schema(schema: Option<&Value>) -> bool {
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
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
        && schema
            .get("additionalProperties")
            .is_none_or(Value::is_boolean)
        && schema.iter().all(|(key, value)| match key.as_str() {
            "type" | "properties" | "required" | "additionalProperties" => true,
            "title" | "description" | "$schema" | "$id" | "$comment" => value.is_string(),
            _ => false,
        })
}

/// Checks the supplied schema without fetching remote or local references.
pub fn validate_schema(params: &Value) -> Result<(), AppServerError> {
    schema::compile(params).map(|_| ())
}

pub(crate) fn response(params: &Value, answer: &str) -> Result<(Value, String), AppServerError> {
    if answer.len() > 65_536 {
        return Err(invalid(
            "MCP input exceeds the 64 KiB input limit; the request remains pending.",
        ));
    }
    let answer = input::unfence(answer)?;
    if matches!(answer, "/cancel" | "/decline") {
        let action = &answer[1..];
        return Ok((
            json!({"action":action,"content":null,"_meta":null}),
            action.into(),
        ));
    }
    let validator = schema::compile(params)?;
    let content = input::parse(&params["requestedSchema"], answer)?;
    schema::check_limits(&content)?;
    if let Err(error) = validator.validate(&content) {
        return Err(invalid(format!(
            "MCP input does not satisfy the schema: {}. Correct the field and reply again; the request remains pending.",
            error.masked()
        )));
    }
    Ok((
        json!({"action":"accept","content":content,"_meta":null}),
        "accept".into(),
    ))
}

fn invalid(message: impl Into<String>) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}

use super::invalid;
use crate::AppServerError;
use serde_json::{Value, json};

pub(super) fn unfence(input: &str) -> Result<&str, AppServerError> {
    let text = input.trim();
    if !text.starts_with("```") {
        return Ok(text);
    }
    let Some((header, rest)) = text.split_once('\n') else {
        return Err(invalid(
            "Use a complete JSON code block or a JSON object. The request remains pending.",
        ));
    };
    if !matches!(header.trim_end_matches('\r'), "```" | "```json") {
        return Err(invalid(
            "Use a JSON code block or a JSON object. The request remains pending.",
        ));
    }
    rest.strip_suffix("\n```").map(str::trim).ok_or_else(|| {
        invalid("Use a complete JSON code block or a JSON object. The request remains pending.")
    })
}

pub(super) fn parse(schema: &Value, answer: &str) -> Result<Value, AppServerError> {
    let parsed = serde_json::from_str::<Value>(answer);
    if parsed.is_err() && answer.starts_with(['{', '[', '"']) {
        return Err(invalid(
            "Invalid JSON input. Correct it and reply again; the request remains pending.",
        ));
    }
    let parsed = parsed.ok();
    if parsed.as_ref().is_some_and(Value::is_object) {
        return Ok(parsed.expect("object checked"));
    }
    let properties = schema.get("properties").and_then(Value::as_object)
        .filter(|fields| fields.len() == 1).ok_or_else(|| invalid(
            "Reply with a JSON object containing the requested field names; the request remains pending."
        ))?;
    let (key, field) = properties.iter().next().expect("one field checked");
    let value = if field.get("type").and_then(Value::as_str) == Some("string") {
        parsed
            .filter(Value::is_string)
            .unwrap_or_else(|| json!(answer))
    } else {
        parsed.ok_or_else(|| invalid(
            "Enter a valid JSON number, true/false, or array for this field; the request remains pending."
        ))?
    };
    Ok(json!({key:value}))
}

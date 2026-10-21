use super::invalid;
use crate::AppServerError;
use serde_json::Value;

pub(super) fn compile(params: &Value) -> Result<jsonschema::Validator, AppServerError> {
    let schema = params
        .get("requestedSchema")
        .filter(|schema| {
            schema.is_object() && schema.get("type").and_then(Value::as_str) == Some("object")
        })
        .ok_or_else(|| invalid("MCP input needs an object schema; the request remains pending."))?;
    check_limits(schema)?;
    check_extensions(schema)?;
    let options = jsonschema::options()
        .offline()
        .should_validate_formats(true);
    let options = if schema.get("$schema").is_none() {
        options.with_draft(jsonschema::Draft::Draft7)
    } else {
        options
    };
    options.build(schema).map_err(|_| invalid(
        "The MCP input schema is invalid or references an unavailable schema; the request remains pending."
    ))
}

fn check_extensions(value: &Value) -> Result<(), AppServerError> {
    match value {
        Value::Object(fields) => {
            if fields.get("$async") == Some(&Value::Bool(true))
                || ["$ref", "$dynamicRef", "$recursiveRef"]
                    .iter()
                    .any(|key| fields.get(*key).is_some_and(Value::is_string))
            {
                return Err(invalid(
                    "MCP forms must use synchronous, self-contained field schemas; the request remains pending.",
                ));
            }
            for child in fields.values() {
                check_extensions(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                check_extensions(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn check_limits(value: &Value) -> Result<(), AppServerError> {
    if value.to_string().len() > 65_536 {
        return Err(invalid(
            "MCP form exceeds the 64 KiB input limit; the request remains pending.",
        ));
    }
    visit(value, 0)
}

fn visit(value: &Value, depth: usize) -> Result<(), AppServerError> {
    if depth > 32 {
        return Err(invalid(
            "MCP form nesting exceeds the supported limit; the request remains pending.",
        ));
    }
    match value {
        Value::Number(number)
            if number
                .as_f64()
                .is_some_and(|v| v.abs() > 9_007_199_254_740_991.0) =>
        {
            return Err(invalid(
                "MCP numeric values must be within the exact integer range; use a string for a large identifier if the form permits it.",
            ));
        }
        Value::Array(values) => {
            for child in values {
                visit(child, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for child in values.values() {
                visit(child, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

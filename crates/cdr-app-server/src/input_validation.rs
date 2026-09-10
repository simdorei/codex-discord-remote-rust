//! Shared structural validation for displaying and answering pending input.
use crate::AppServerError;
use serde_json::Value;
use std::collections::BTreeSet;

pub fn validate_input_questions(params: &Value) -> Result<&[Value], AppServerError> {
    let questions = params
        .get("questions")
        .and_then(Value::as_array)
        .filter(|questions| !questions.is_empty())
        .ok_or_else(|| invalid("No pending input questions were available."))?;
    let mut ids = BTreeSet::new();
    for question in questions {
        let id = question
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("Pending input question did not include an id."))?;
        if !ids.insert(id) {
            return Err(invalid("Pending input contained duplicate question ids."));
        }
        input_option_labels(question)?;
    }
    Ok(questions)
}

pub fn input_option_labels(question: &Value) -> Result<Vec<&str>, AppServerError> {
    let Some(options) = question.get("options").filter(|options| !options.is_null()) else {
        return Ok(Vec::new());
    };
    options
        .as_array()
        .ok_or_else(|| invalid("Pending input options were not an array."))?
        .iter()
        .map(|option| {
            option
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .ok_or_else(|| invalid("Pending input option had a missing or empty label."))
        })
        .collect()
}

fn invalid(message: &str) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.to_owned(),
    }
}

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::AppServerError;

#[derive(Debug, Clone, PartialEq)]
pub struct InputResponse {
    pub payload: Value,
    pub answers_by_question: BTreeMap<String, Vec<String>>,
}

#[must_use]
pub fn split_input_values(raw: &str) -> Vec<String> {
    raw.split('|')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn resolve_input_answers(question: &Value, raw: &str) -> Result<Vec<String>, AppServerError> {
    let values = split_input_values(raw);
    if values.is_empty() {
        return Err(invalid("Answer text was empty."));
    }
    let labels = crate::input_option_labels(question)?;
    Ok(values
        .into_iter()
        .map(|value| {
            if let Ok(index) = value.parse::<usize>()
                && let Some(label) = index.checked_sub(1).and_then(|index| labels.get(index))
            {
                return (*label).to_owned();
            }
            labels
                .iter()
                .find(|label| label.eq_ignore_ascii_case(&value))
                .map_or(value, |label| (*label).to_owned())
        })
        .collect())
}

pub fn build_input_response(params: &Value, answer: &str) -> Result<InputResponse, AppServerError> {
    let questions = crate::validate_input_questions(params)?;
    let mut question_map = BTreeMap::new();
    for (index, question) in questions.iter().enumerate() {
        if !question.is_object() {
            return Err(invalid(format!(
                "Pending input question {} was invalid.",
                index + 1
            )));
        }
        let id = question
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                invalid(format!(
                    "Pending input question {} did not include an id.",
                    index + 1
                ))
            })?;
        question_map.insert(id.to_owned(), question);
    }
    let normalized = answer.trim();
    if normalized.is_empty() {
        return Err(invalid("Answer text was empty."));
    }
    let assignments = parse_assignments(&question_map, normalized)?;
    let missing: Vec<_> = question_map
        .keys()
        .filter(|id| !assignments.contains_key(*id))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Err(invalid(format!(
            "Missing answers for question ids: {}",
            missing.join(", ")
        )));
    }
    let unknown: Vec<_> = assignments
        .keys()
        .filter(|id| !question_map.contains_key(*id))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(invalid(format!(
            "Unknown question ids: {}",
            unknown.join(", ")
        )));
    }
    let mut answers = BTreeMap::new();
    let mut payload = Map::new();
    for (id, raw) in assignments {
        let values = resolve_input_answers(question_map[&id], &raw)?;
        payload.insert(id.clone(), json!({"answers": values}));
        answers.insert(id, values);
    }
    Ok(InputResponse {
        payload: json!({"answers": payload}),
        answers_by_question: answers,
    })
}

fn parse_assignments(
    questions: &BTreeMap<String, &Value>,
    normalized: &str,
) -> Result<BTreeMap<String, String>, AppServerError> {
    let mut assignments = BTreeMap::new();
    if questions.len() == 1 && !normalized.contains('=') {
        let id = questions.first_key_value().expect("one question").0.clone();
        assignments.insert(id, normalized.to_owned());
        return Ok(assignments);
    }
    for segment in normalized
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        let (id, value) = segment.split_once('=').ok_or_else(|| {
            invalid("Multi-question replies must use question_id=value; other_id=value format.")
        })?;
        if id.trim().is_empty() {
            return Err(invalid(
                "A reply_input assignment was missing the question id.",
            ));
        }
        assignments.insert(id.trim().to_owned(), value.trim().to_owned());
    }
    Ok(assignments)
}

fn invalid(message: impl Into<String>) -> AppServerError {
    AppServerError::InvalidReply {
        message: message.into(),
    }
}

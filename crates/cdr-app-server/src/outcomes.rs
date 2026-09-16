use std::collections::BTreeMap;

use crate::is_usage_limit_error;
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStatus {
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptOrigin {
    RemoteUserIntent,
    ExternalOrUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnCompletion {
    pub thread_id: String,
    pub turn_id: String,
    pub status: TurnStatus,
    pub error_message: String,
    pub interrupt_origin: Option<InterruptOrigin>,
    pub duration_ms: Option<i64>,
    pub usage_limit: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedFinalAnswer {
    pub thread_id: String,
    pub turn_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TurnOutcomeError {
    #[error("thread/read returned an invalid thread payload")]
    InvalidThread,
    #[error("thread/read returned a different thread")]
    DifferentThread,
    #[error("thread/read returned invalid turns")]
    InvalidTurns,
    #[error("thread/read returned an invalid turn payload")]
    InvalidTurn,
    #[error("turn payload had no thread id")]
    MissingThreadId,
    #[error("turn payload had no turn id")]
    MissingTurnId,
    #[error("turn payload had no status")]
    MissingStatus,
    #[error("turn payload had an unknown status: {0}")]
    UnknownStatus(String),
    #[error("turn/completed carried an inProgress turn")]
    InProgressCompletion,
    #[error("turn payload had an invalid error")]
    InvalidError,
    #[error("turn payload error had no message")]
    MissingErrorMessage,
    #[error("thread/read did not contain the requested turn")]
    TurnNotFound,
    #[error("thread/read returned invalid turn items")]
    InvalidItems,
}

pub fn parse_turn_completion(
    params: &Value,
    remote_user_intent: bool,
) -> Result<TurnCompletion, TurnOutcomeError> {
    let thread_id = text(params.get("threadId"));
    let turn = params
        .get("turn")
        .and_then(Value::as_object)
        .ok_or(TurnOutcomeError::InvalidTurn)?;
    parse_turn_payload(
        &thread_id,
        &Value::Object(turn.clone()),
        remote_user_intent,
        true,
    )
}

pub fn parse_thread_turn_states(
    result: &Value,
    expected_thread_id: &str,
) -> Result<BTreeMap<String, TurnCompletion>, TurnOutcomeError> {
    let thread = result
        .get("thread")
        .and_then(Value::as_object)
        .ok_or(TurnOutcomeError::InvalidThread)?;
    let thread_id = text(thread.get("id"));
    if thread_id != expected_thread_id {
        return Err(TurnOutcomeError::DifferentThread);
    }
    let turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .ok_or(TurnOutcomeError::InvalidTurns)?;
    let mut states = BTreeMap::new();
    for turn in turns {
        if !turn.is_object() {
            return Err(TurnOutcomeError::InvalidTurn);
        }
        let completion = parse_turn_payload(&thread_id, turn, false, false)?;
        states.insert(completion.turn_id.clone(), completion);
    }
    Ok(states)
}

/// Preserve whether history contains an explicit final answer or only a legacy
/// last-agent/empty fallback. Callers must not let weaker text displace a journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnText {
    pub text: String,
    pub explicit_final: bool,
}

pub fn extract_turn_final_text(
    result: &Value,
    expected_thread_id: &str,
    expected_turn_id: &str,
) -> Result<String, TurnOutcomeError> {
    Ok(extract_turn_text(result, expected_thread_id, expected_turn_id)?.text)
}

pub fn extract_turn_text(
    result: &Value,
    expected_thread_id: &str,
    expected_turn_id: &str,
) -> Result<TurnText, TurnOutcomeError> {
    let thread = result
        .get("thread")
        .and_then(Value::as_object)
        .ok_or(TurnOutcomeError::InvalidThread)?;
    if text(thread.get("id")) != expected_thread_id {
        return Err(TurnOutcomeError::DifferentThread);
    }
    let turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .ok_or(TurnOutcomeError::InvalidTurns)?;
    let turn = turns
        .iter()
        .find(|turn| text(turn.get("id")) == expected_turn_id)
        .ok_or(TurnOutcomeError::TurnNotFound)?;
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .ok_or(TurnOutcomeError::InvalidItems)?;
    let mut fallback = String::new();
    let mut final_answer = String::new();
    for item in items {
        if !matches!(
            item.get("type").and_then(Value::as_str),
            Some("agentMessage" | "agent_message")
        ) {
            continue;
        }
        let message = agent_message_text(item);
        if message.is_empty() {
            continue;
        }
        message.clone_into(&mut fallback);
        if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
            message.clone_into(&mut final_answer);
        }
    }
    let explicit_final = !final_answer.is_empty();
    Ok(TurnText {
        text: if explicit_final {
            final_answer
        } else {
            fallback
        },
        explicit_final,
    })
}

#[must_use]
pub fn extract_completed_final_answer(params: &Value) -> Option<CompletedFinalAnswer> {
    let item = params.get("item")?;
    if !matches!(
        item.get("type").and_then(Value::as_str),
        Some("agentMessage" | "agent_message")
    ) || item.get("phase").and_then(Value::as_str) != Some("final_answer")
    {
        return None;
    }
    let thread_id = text(params.get("threadId"));
    let turn_id = text(params.get("turnId"));
    let text = agent_message_text(item);
    if thread_id.is_empty() || turn_id.is_empty() || text.is_empty() {
        return None;
    }
    Some(CompletedFinalAnswer {
        thread_id,
        turn_id,
        text,
    })
}

fn agent_message_text(item: &Value) -> String {
    let direct = text(item.get("text"));
    if !direct.is_empty() {
        return direct;
    }
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| {
            matches!(
                block.get("type").and_then(Value::as_str),
                Some("output_text" | "text")
            )
        })
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn parse_turn_payload(
    thread_id: &str,
    turn: &Value,
    remote_user_intent: bool,
    require_terminal: bool,
) -> Result<TurnCompletion, TurnOutcomeError> {
    if thread_id.is_empty() {
        return Err(TurnOutcomeError::MissingThreadId);
    }
    let turn_id = text(turn.get("id"));
    if turn_id.is_empty() {
        return Err(TurnOutcomeError::MissingTurnId);
    }
    let raw_status = turn
        .get("status")
        .and_then(Value::as_str)
        .ok_or(TurnOutcomeError::MissingStatus)?;
    let status = match raw_status {
        "completed" => TurnStatus::Completed,
        "interrupted" => TurnStatus::Interrupted,
        "failed" => TurnStatus::Failed,
        "inProgress" => TurnStatus::InProgress,
        other => return Err(TurnOutcomeError::UnknownStatus(other.into())),
    };
    if require_terminal && status == TurnStatus::InProgress {
        return Err(TurnOutcomeError::InProgressCompletion);
    }
    let (error_message, usage_limit) = parse_error(turn, status)?;
    let interrupt_origin = (status == TurnStatus::Interrupted).then_some(if remote_user_intent {
        InterruptOrigin::RemoteUserIntent
    } else {
        InterruptOrigin::ExternalOrUnknown
    });
    Ok(TurnCompletion {
        thread_id: thread_id.into(),
        turn_id,
        status,
        error_message,
        interrupt_origin,
        duration_ms: turn.get("durationMs").and_then(Value::as_i64),
        usage_limit,
    })
}

fn parse_error(turn: &Value, status: TurnStatus) -> Result<(String, bool), TurnOutcomeError> {
    let Some(error) = turn.get("error") else {
        return Ok((String::new(), false));
    };
    if error.is_null() {
        return Ok((String::new(), false));
    }
    let error = error.as_object().ok_or(TurnOutcomeError::InvalidError)?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .ok_or(TurnOutcomeError::MissingErrorMessage)?;
    if status != TurnStatus::Failed {
        return Ok((String::new(), false));
    }
    let usage_limit = is_usage_limit_error(Some(&Value::Object(error.clone())));
    Ok((message.trim().chars().take(1_000).collect(), usage_limit))
}

/// Bounded terminal metadata for durable recovery; no arbitrary additionalDetails.
#[must_use]
pub fn completion_journal_payload(completion: &TurnCompletion) -> Value {
    let status = match completion.status {
        TurnStatus::Completed => "completed",
        TurnStatus::Interrupted => "interrupted",
        TurnStatus::Failed => "failed",
        TurnStatus::InProgress => "inProgress",
    };
    serde_json::json!({"threadId":completion.thread_id,"turn":{
        "id":completion.turn_id,"status":status,"durationMs":completion.duration_ms,
        "error":{"message":completion.error_message,"codexErrorInfo":
            if completion.status == TurnStatus::Failed && completion.usage_limit {
                Some("usageLimitExceeded")
            } else { None }
        }
    }})
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

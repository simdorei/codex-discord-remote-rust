use serde_json::Value;

use super::{
    FinalWatchState, GoalLookup, RolloutTerminal, WatchResult, WatchStatus, complete_for_goal,
    result,
};

pub fn process_events(
    state: &mut FinalWatchState,
    events: &[Value],
    expected_turn_id: Option<&str>,
    goal: &GoalLookup,
) -> Option<WatchResult> {
    for event in events {
        let Some(payload) = event.get("payload").and_then(Value::as_object) else {
            continue;
        };
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if event_type == "response_item" {
            process_response_item(state, payload_type, &Value::Object(payload.clone()));
        } else if event_type == "event_msg"
            && let Some(outcome) = process_event_message(
                state,
                payload_type,
                &Value::Object(payload.clone()),
                expected_turn_id,
                goal,
            )
        {
            return Some(outcome);
        }
    }
    None
}

fn process_event_message(
    state: &mut FinalWatchState,
    kind: &str,
    payload: &Value,
    expected_turn_id: Option<&str>,
    goal: &GoalLookup,
) -> Option<WatchResult> {
    match kind {
        "agent_message" => {
            let message = text(payload.get("message"));
            if payload.get("phase").and_then(Value::as_str) == Some("final_answer") {
                state.final_candidate = message;
            } else {
                append_commentary(state, message, true);
            }
            None
        }
        "turn_aborted" | "task_aborted" | "task_cancelled" => {
            if expected_turn_id.is_some() {
                record_terminal(state, kind, payload, expected_turn_id);
                None
            } else {
                state.final_candidate.clear();
                Some(result(state, WatchStatus::Aborted, "", "", None))
            }
        }
        "task_complete" => {
            if expected_turn_id.is_some() {
                record_terminal(state, kind, payload, expected_turn_id);
                None
            } else {
                let answer = nonempty(payload.get("last_agent_message"))
                    .unwrap_or_else(|| state.final_candidate.clone());
                Some(complete_for_goal(state, answer, goal))
            }
        }
        _ => None,
    }
}

fn process_response_item(state: &mut FinalWatchState, kind: &str, payload: &Value) {
    match kind {
        "message" if payload.get("role").and_then(Value::as_str) == Some("assistant") => {
            let message = message_text(payload);
            match payload.get("phase").and_then(Value::as_str) {
                Some("final_answer") => state.final_candidate = message,
                Some("commentary") => append_commentary(state, message, false),
                _ => {}
            }
        }
        "function_call_output" => {
            let output = text(payload.get("output"));
            if output.to_ascii_lowercase().contains("rejected by user") {
                append_interactive(
                    state,
                    "[approval_rejected]\nCommand approval was rejected by user.".into(),
                );
            }
        }
        "function_call" => append_interactive(state, interactive_notice(payload)),
        _ => {}
    }
}

fn record_terminal(
    state: &mut FinalWatchState,
    kind: &str,
    payload: &Value,
    expected: Option<&str>,
) {
    let payload_turn = text(payload.get("turn_id"));
    if !payload_turn.is_empty() && Some(payload_turn.as_str()) != expected {
        return;
    }
    state.rollout_terminal = Some(RolloutTerminal {
        kind: kind.into(),
        last_agent_message: text(payload.get("last_agent_message")),
    });
}

pub(super) fn append_commentary(state: &mut FinalWatchState, message: String, dedupe: bool) {
    if message.is_empty() || (dedupe && !state.seen_agent_messages.insert(message.clone())) {
        return;
    }
    if !dedupe && state.commentary.last() == Some(&message) {
        return;
    }
    state.commentary.push(message);
}

fn append_interactive(state: &mut FinalWatchState, notice: String) {
    if !notice.is_empty() && state.seen_interactive_notices.insert(notice.clone()) {
        state.commentary.push(notice);
    }
}

fn interactive_notice(payload: &Value) -> String {
    let name = text(payload.get("name"));
    let arguments = payload
        .get("arguments")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if name == "request_user_input" {
        return "[choice_required]".into();
    }
    if arguments.get("sandbox_permissions").and_then(Value::as_str) == Some("require_escalated") {
        return format!("[approval_required]\ntool: {name}");
    }
    String::new()
}

fn message_text(payload: &Value) -> String {
    payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    let value = text(value);
    (!value.is_empty()).then_some(value)
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .into()
}

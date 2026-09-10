use std::collections::HashSet;

use serde_json::{Map, Value};

use super::helpers::{
    message_text, push_activity_item, push_item, push_item_with_turn, push_user, string,
    visible_texts,
};
use super::{MirrorDetail, MirrorItem, MirrorKind, SessionEvent};

#[must_use]
pub fn collect_items(
    thread_id: &str,
    events: &[SessionEvent],
    detail: MirrorDetail,
) -> Vec<MirrorItem> {
    collect_items_with_context(thread_id, events, detail, None).0
}

pub(crate) fn collect_items_with_context(
    thread_id: &str,
    events: &[SessionEvent],
    detail: MirrorDetail,
    mut current_turn: Option<String>,
) -> (Vec<MirrorItem>, Option<String>) {
    let mut items = Vec::new();
    let mut terminal_turns = HashSet::new();
    for event in events {
        if let Some(turn) = super::turn_context::turn_from_event(event) {
            current_turn = Some(turn.to_owned());
        }
        let Some(payload) = event.get("payload").and_then(Value::as_object) else {
            continue;
        };
        // Parse one event in isolation: inherited turn identity must be attached
        // before comparing it with records from other events or previous polls.
        let mut event_items = Vec::new();
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "event_msg" => {
                collect_event_message(
                    thread_id,
                    event,
                    payload,
                    &mut terminal_turns,
                    &mut event_items,
                );
            }
            "response_item" => {
                collect_response_item(thread_id, event, payload, detail, &mut event_items);
            }
            _ => {}
        }
        for mut item in event_items {
            if item.turn_id.is_none() {
                item.turn_id.clone_from(&current_turn);
                if let Some(turn) = item.turn_id.as_deref() {
                    item.digest = super::helpers::turn_scoped_digest(turn, &item.digest);
                }
            }
            if !items
                .iter()
                .any(|prior: &MirrorItem| prior.digest == item.digest)
            {
                items.push(item);
            }
        }
    }
    (items, current_turn)
}

fn collect_event_message(
    thread_id: &str,
    event: &Map<String, Value>,
    payload: &Map<String, Value>,
    terminal_turns: &mut HashSet<String>,
    items: &mut Vec<MirrorItem>,
) {
    let payload_type = string(payload, "type");
    match payload_type {
        "agent_message" => {
            let raw_phase = string(payload, "phase");
            let phase = if raw_phase.is_empty() {
                "commentary"
            } else {
                raw_phase
            };
            if phase != "final_answer" {
                push_item(
                    thread_id,
                    event,
                    MirrorKind::Commentary,
                    phase,
                    string(payload, "message"),
                    items,
                );
            }
        }
        "user_message" => push_user(thread_id, event, string(payload, "message"), "input", items),
        "task_complete" => collect_complete(thread_id, event, payload, terminal_turns, items),
        "turn_aborted" | "task_aborted" | "task_cancelled" => {
            collect_aborted(thread_id, event, payload, terminal_turns, items);
        }
        _ => {}
    }
}

fn collect_complete(
    thread_id: &str,
    event: &Map<String, Value>,
    payload: &Map<String, Value>,
    terminal_turns: &mut HashSet<String>,
    items: &mut Vec<MirrorItem>,
) {
    let turn_id = string(payload, "turn_id");
    if !turn_id.is_empty() && !terminal_turns.insert(turn_id.into()) {
        return;
    }
    if let Some(error) = payload.get("error").filter(|value| !value.is_null()) {
        let raw = error
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| error.to_string(), str::to_owned);
        push_item_with_turn(
            thread_id,
            event,
            MirrorKind::Failed,
            "error",
            &crate::error_message::readable_error(&raw),
            Some(turn_id),
            items,
        );
        return;
    }
    let text = string(payload, "last_agent_message");
    let text = if text.is_empty() {
        "Codex turn completed without a visible reply."
    } else {
        text
    };
    push_item_with_turn(
        thread_id,
        event,
        MirrorKind::Final,
        "final_answer",
        text,
        Some(turn_id),
        items,
    );
}

fn collect_aborted(
    thread_id: &str,
    event: &Map<String, Value>,
    payload: &Map<String, Value>,
    terminal_turns: &mut HashSet<String>,
    items: &mut Vec<MirrorItem>,
) {
    let turn_id = string(payload, "turn_id");
    if !turn_id.is_empty() && !terminal_turns.insert(turn_id.into()) {
        return;
    }
    let payload_type = string(payload, "type");
    let headline = match payload_type {
        "task_cancelled" => "Codex task cancelled.",
        "task_aborted" => "Codex task aborted.",
        _ => "Codex turn aborted.",
    };
    push_item_with_turn(
        thread_id,
        event,
        MirrorKind::Aborted,
        payload_type,
        headline,
        Some(turn_id),
        items,
    );
}

fn collect_response_item(
    thread_id: &str,
    event: &Map<String, Value>,
    payload: &Map<String, Value>,
    detail: MirrorDetail,
    items: &mut Vec<MirrorItem>,
) {
    let payload_type = string(payload, "type");
    if payload_type == "message" {
        collect_response_message(thread_id, event, payload, items);
        return;
    }
    if detail != MirrorDetail::All {
        return;
    }
    match payload_type {
        "reasoning" => {
            for (index, text) in visible_texts(payload.get("summary")).iter().enumerate() {
                push_activity_item(thread_id, event, "reasoning", text, index, items);
            }
        }
        "function_call" | "custom_tool_call" => {
            let name = string(payload, "name");
            let text = if name.is_empty() {
                "Tool call".into()
            } else {
                format!("Tool call: {name}")
            };
            push_activity_item(thread_id, event, "tool_call", &text, 0, items);
        }
        "function_call_output" | "custom_tool_call_output" => {
            for (index, text) in visible_texts(payload.get("output")).iter().enumerate() {
                push_activity_item(
                    thread_id,
                    event,
                    "tool_output",
                    &format!("Tool output:\n{text}"),
                    index,
                    items,
                );
            }
        }
        _ => {}
    }
}

fn collect_response_message(
    thread_id: &str,
    event: &Map<String, Value>,
    payload: &Map<String, Value>,
    items: &mut Vec<MirrorItem>,
) {
    let role = string(payload, "role");
    let phase = string(payload, "phase");
    let text = message_text(payload);
    if role == "assistant" && phase == "commentary" {
        push_item(
            thread_id,
            event,
            MirrorKind::Commentary,
            phase,
            &text,
            items,
        );
    } else if role == "user" {
        push_user(thread_id, event, &text, phase, items);
    }
}

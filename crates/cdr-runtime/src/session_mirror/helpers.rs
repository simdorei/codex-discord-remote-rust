use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{MirrorItem, MirrorKind};

const INTERNAL_USER_PREFIXES: [&str; 4] = [
    "# AGENTS.md instructions",
    "<INSTRUCTIONS>",
    "<environment_context",
    "<codex_internal_context",
];

pub(super) fn push_user(
    thread_id: &str,
    event: &Map<String, Value>,
    text: &str,
    phase: &str,
    items: &mut Vec<MirrorItem>,
) {
    if !INTERNAL_USER_PREFIXES
        .iter()
        .any(|prefix| text.trim_start().starts_with(prefix))
    {
        push_item(thread_id, event, MirrorKind::User, phase, text, items);
    }
}

pub(super) fn push_item(
    thread_id: &str,
    event: &Map<String, Value>,
    kind: MirrorKind,
    phase: &str,
    text: &str,
    items: &mut Vec<MirrorItem>,
) {
    push_item_with_turn(thread_id, event, kind, phase, text, None, items);
}

pub(super) fn push_item_with_turn(
    thread_id: &str,
    event: &Map<String, Value>,
    kind: MirrorKind,
    phase: &str,
    text: &str,
    turn_id: Option<&str>,
    items: &mut Vec<MirrorItem>,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let digest = if matches!(
        kind,
        MirrorKind::Final | MirrorKind::Failed | MirrorKind::Aborted
    ) && let Some(turn) = turn_id.filter(|value| !value.is_empty())
    {
        text_digest(["session-terminal-v2", thread_id, turn])
    } else {
        event_digest(thread_id, event, kind, phase, text)
    };
    if !items.iter().any(|item| item.digest == digest) {
        items.push(MirrorItem {
            digest,
            kind,
            phase: phase.into(),
            text: text.into(),
            turn_id: turn_id.filter(|value| !value.is_empty()).map(str::to_owned),
            dedupe_recent_text: kind == MirrorKind::Commentary,
        });
    }
}

pub(super) fn push_activity_item(
    thread_id: &str,
    event: &Map<String, Value>,
    phase: &str,
    text: &str,
    item_index: usize,
    items: &mut Vec<MirrorItem>,
) {
    let clean_text = text.trim();
    if clean_text.is_empty() {
        return;
    }
    let base = event_digest(thread_id, event, MirrorKind::Commentary, phase, clean_text);
    let digest = text_digest([base.as_str(), text, &item_index.to_string()]);
    items.push(MirrorItem {
        digest,
        kind: MirrorKind::Commentary,
        phase: phase.into(),
        text: text.into(),
        turn_id: None,
        dedupe_recent_text: false,
    });
}

fn event_digest(
    thread_id: &str,
    event: &Map<String, Value>,
    kind: MirrorKind,
    phase: &str,
    text: &str,
) -> String {
    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if kind == MirrorKind::User {
        return text_digest(["session-mirror", thread_id, timestamp, "user", text]);
    }
    let (kind, role) = match kind {
        MirrorKind::User => ("user", "user"),
        MirrorKind::Commentary => ("commentary", "assistant"),
        MirrorKind::Final => ("final", "assistant"),
        MirrorKind::Aborted => ("aborted", "assistant"),
        MirrorKind::Failed => ("failed", "assistant"),
    };
    let payload_type = event
        .get("payload")
        .and_then(Value::as_object)
        .and_then(|payload| payload.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    text_digest([
        "session-mirror",
        thread_id,
        timestamp,
        event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        payload_type,
        kind,
        role,
        phase,
        text,
    ])
}

pub(super) fn turn_scoped_digest(turn: &str, event_digest: &str) -> String {
    text_digest(["session-turn-event-v1", turn, event_digest])
}

fn text_digest<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    hex::encode(digest.finalize())
}

pub(super) fn message_text(payload: &Map<String, Value>) -> String {
    payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|part| matches!(string(part, "type"), "input_text" | "output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}

pub(super) fn visible_texts(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => vec![text.clone()],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| part.as_str().map(str::to_owned))
            })
            .filter(|text| !text.is_empty())
            .collect(),
        Some(value) if !value.is_null() => vec![value.to_string()],
        _ => Vec::new(),
    }
}

pub(super) fn string<'a>(payload: &'a Map<String, Value>, field: &str) -> &'a str {
    payload
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

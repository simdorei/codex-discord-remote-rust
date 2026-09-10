from __future__ import annotations

from collections import deque
from pathlib import Path

from codex_desktop_bridge_interactive_payloads import (
    JsonObject,
    build_interactive_notice_from_function_call,
    classify_interactive_function_call,
    coerce_json_object,
    extract_message_text,
    parse_function_call_arguments,
    summarize_interactive_lines,
)
from codex_session_events import iter_session_events

__all__ = [
    "JsonObject",
    "build_interactive_notice_from_function_call",
    "classify_interactive_function_call",
    "extract_message_text",
    "get_last_user_and_assistant_messages",
    "get_pending_interactive_display_lines",
    "get_pending_interactive_function_call_from_session",
    "get_pending_interactive_state_from_session",
    "get_pending_interactive_summary",
    "get_pending_permission_approval_from_session",
    "parse_function_call_arguments",
    "summarize_interactive_lines",
]


def get_pending_interactive_function_call_from_session(
    session_path: Path,
    *,
    recent_limit: int = 256,
) -> JsonObject | None:
    recent_events: deque[JsonObject] = deque(maxlen=max(32, recent_limit))
    try:
        for event in iter_session_events(session_path):
            recent_events.append(coerce_json_object(event))
    except OSError:
        return None

    completed_call_ids: set[str] = set()
    for event in reversed(recent_events):
        if event.get("type") != "response_item":
            continue
        payload = coerce_json_object(event.get("payload"))

        payload_type = str(payload.get("type") or "").strip()
        if payload_type == "function_call_output":
            call_id = str(payload.get("call_id") or "").strip()
            if call_id:
                completed_call_ids.add(call_id)
            continue
        if payload_type != "function_call":
            continue

        state = classify_interactive_function_call(payload)
        if not state:
            continue

        call_id = str(payload.get("call_id") or "").strip()
        if call_id and call_id in completed_call_ids:
            continue
        return payload

    return None


def get_pending_interactive_state_from_session(session_path: Path, *, recent_limit: int = 256) -> str | None:
    payload = get_pending_interactive_function_call_from_session(
        session_path,
        recent_limit=recent_limit,
    )
    return classify_interactive_function_call(payload)


def get_pending_permission_approval_from_session(
    session_path: Path,
    *,
    recent_limit: int = 256,
) -> JsonObject | None:
    payload = get_pending_interactive_function_call_from_session(
        session_path,
        recent_limit=recent_limit,
    )
    if classify_interactive_function_call(payload) != "waiting-approval":
        return None
    if payload is None:
        return None
    args = parse_function_call_arguments(payload)
    if str(args.get("sandbox_permissions") or "").strip().lower() != "require_escalated":
        return None
    return {
        "call_id": str(payload.get("call_id") or "").strip(),
        "tool_name": str(payload.get("name") or "").strip(),
        "question": str(args.get("justification") or "").strip(),
    }


def get_pending_interactive_display_lines(
    session_path: Path,
    *,
    recent_limit: int = 256,
) -> tuple[str | None, list[str]]:
    payload = get_pending_interactive_function_call_from_session(
        session_path,
        recent_limit=recent_limit,
    )
    state = classify_interactive_function_call(payload)
    if not state or payload is None:
        return None, []
    notice = build_interactive_notice_from_function_call(payload)
    lines = [line.strip() for line in notice.splitlines() if line.strip()]
    if lines and lines[0].startswith("[") and lines[0].endswith("]"):
        lines = lines[1:]
    return state, lines


def get_pending_interactive_summary(session_path: Path, *, limit: int = 100) -> str:
    state, lines = get_pending_interactive_display_lines(session_path)
    return summarize_interactive_lines(state, lines, limit=limit)


def get_last_user_and_assistant_messages(session_path: Path) -> tuple[str, str]:
    last_user = ""
    last_assistant = ""

    for event in iter_session_events(session_path):
        event_payload = coerce_json_object(event)
        payload = coerce_json_object(event_payload.get("payload"))
        if event_payload.get("type") == "response_item" and payload.get("type") == "message":
            text = extract_message_text(payload)
            if payload.get("role") == "user" and text:
                last_user = text
            if payload.get("role") == "assistant" and text:
                last_assistant = text

    return last_user, last_assistant

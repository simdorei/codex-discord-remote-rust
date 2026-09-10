from __future__ import annotations

from collections.abc import Callable

from codex_session_events import JsonEvent, JsonValue


ExtractMessageTextFunc = Callable[[dict[str, JsonValue]], str]


def extract_user_text_from_session_event(
    event: JsonEvent,
    *,
    extract_message_text_func: ExtractMessageTextFunc,
) -> str:
    payload = event.get("payload") or {}
    if not isinstance(payload, dict):
        return ""
    if event.get("type") == "event_msg" and payload.get("type") == "user_message":
        return str(payload.get("message") or "").strip()
    if event.get("type") != "response_item":
        return ""
    if payload.get("type") != "message" or payload.get("role") != "user":
        return ""
    return extract_message_text_func(payload).strip()

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Final

from codex_discord_session_mirror_item_append import SessionPayload
from codex_session_events import JsonValue

TOOL_CALL_TYPES: Final = frozenset({"function_call", "custom_tool_call"})
TOOL_OUTPUT_TYPES: Final = frozenset(
    {"function_call_output", "custom_tool_call_output"}
)
ATTACHMENT_PART_TYPES: Final = frozenset(
    {"file", "input_file", "input_image", "output_file"}
)


@dataclass(frozen=True, slots=True)
class ActivityItem:
    phase: str
    text: str


def build_activity_items(payload: SessionPayload) -> tuple[ActivityItem, ...]:
    payload_type = str(payload.get("type") or "")
    if payload_type == "reasoning":
        return tuple(
            ActivityItem(phase="reasoning", text=text)
            for text in _visible_text_parts(payload.get("summary"))
        )
    if payload_type in TOOL_CALL_TYPES:
        return (_tool_call_item(payload, payload_type),)
    if payload_type in TOOL_OUTPUT_TYPES:
        return tuple(
            ActivityItem(phase="tool_output", text=f"Tool output:\n{text}")
            for text in _visible_text_parts(payload.get("output"))
        )
    return ()


def _tool_call_item(payload: SessionPayload, payload_type: str) -> ActivityItem:
    name = str(payload.get("name") or "").strip()
    header = f"Tool call: {name}" if name else "Tool call"
    input_field = "arguments" if payload_type == "function_call" else "input"
    input_text = _visible_value_text(payload.get(input_field))
    text = f"{header}\nInput:\n{input_text}" if input_text else header
    return ActivityItem(phase="tool_call", text=text)


def _visible_text_parts(value: JsonValue | None) -> tuple[str, ...]:
    if isinstance(value, str):
        return (value,) if value else ()
    if not isinstance(value, list):
        text = _visible_value_text(value)
        return (text,) if text else ()

    texts: list[str] = []
    for part in value:
        if isinstance(part, dict):
            if str(part.get("type") or "") in ATTACHMENT_PART_TYPES:
                continue
            text = part.get("text")
            if isinstance(text, str):
                if text:
                    texts.append(text)
                continue
        text = _visible_value_text(part)
        if text:
            texts.append(text)
    return tuple(texts)


def _visible_value_text(value: JsonValue | None) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, indent=2)

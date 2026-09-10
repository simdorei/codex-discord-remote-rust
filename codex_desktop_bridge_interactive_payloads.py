from __future__ import annotations

from collections.abc import Callable
import json
import re
from typing import TypeAlias

from codex_session_events import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]
_decode_json_value: Callable[[str], JsonValue] = json.loads

__all__ = [
    "JsonObject",
    "build_interactive_notice_from_function_call",
    "classify_interactive_function_call",
    "coerce_json_object",
    "extract_message_text",
    "parse_function_call_arguments",
    "summarize_interactive_lines",
]


def extract_message_text(payload: JsonObject) -> str:
    parts = payload.get("content")
    if not isinstance(parts, list):
        return ""
    texts: list[str] = []
    for part in parts:
        part_payload = coerce_json_object(part)
        if part_payload.get("type") not in ("input_text", "output_text"):
            continue
        text = str(part_payload.get("text") or "")
        if text:
            texts.append(text)
    return "\n".join(texts).strip()


def parse_function_call_arguments(payload: JsonObject) -> JsonObject:
    raw = payload.get("arguments")
    if isinstance(raw, dict):
        return raw.copy()
    if not isinstance(raw, str):
        return {}
    raw = raw.strip()
    if not raw:
        return {}
    try:
        parsed = _decode_json_value(raw)
    except json.JSONDecodeError:
        return {}
    return coerce_json_object(parsed)


def build_interactive_notice_from_function_call(payload: JsonObject) -> str:
    name = str(payload.get("name") or "").strip()
    args = parse_function_call_arguments(payload)

    if name == "request_user_input":
        lines = ["[choice_required]"]
        questions = args.get("questions")
        if isinstance(questions, list) and questions:
            first = coerce_json_object(questions[0])
            prompt = str(first.get("question") or "").strip()
            if prompt:
                lines.append(prompt)
            options = first.get("options")
            if isinstance(options, list):
                for index, option in enumerate(options, start=1):
                    option_payload = coerce_json_object(option)
                    label = str(option_payload.get("label") or "").strip()
                    if label:
                        lines.append(f"{index}. {label}")
        return "\n".join(lines)

    if str(args.get("sandbox_permissions") or "").strip().lower() == "require_escalated":
        lines = ["[approval_required]"]
        tool_name = name or str(args.get("tool") or "").strip()
        if tool_name:
            lines.append(f"tool: {tool_name}")
        question = str(args.get("justification") or "").strip()
        if question:
            lines.append(question)
        return "\n".join(lines)

    return ""


def classify_interactive_function_call(payload: JsonObject | None) -> str | None:
    if payload is None:
        return None
    name = str(payload.get("name") or "").strip()
    args = parse_function_call_arguments(payload)
    if name == "request_user_input":
        return "waiting-input"
    if str(args.get("sandbox_permissions") or "").strip().lower() == "require_escalated":
        return "waiting-approval"
    return None


def summarize_interactive_lines(
    state: str | None,
    lines: list[str],
    *,
    limit: int = 100,
) -> str:
    if not state or not lines:
        return ""
    if state == "waiting-approval":
        if len(lines) >= 2 and lines[0].lower().startswith("tool:"):
            return _collapse_list_text(f"{lines[0]} | {lines[1]}", limit=limit)
        return _collapse_list_text(lines[0], limit=limit)
    question = lines[0]
    option_lines = [line for line in lines[1:] if re.match(r"^\d+[\.\)]\s+", line)]
    if option_lines:
        return _collapse_list_text(f"{question} | {' / '.join(option_lines[:2])}", limit=limit)
    return _collapse_list_text(question, limit=limit)


def coerce_json_object(value: JsonValue | None) -> JsonObject:
    if not isinstance(value, dict):
        return {}
    return value.copy()


def _collapse_list_text(value: str, limit: int = 70) -> str:
    text = " ".join(value.split())
    if len(text) <= limit:
        return text
    return text[: max(0, limit - 1)].rstrip() + "…"

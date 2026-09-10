from __future__ import annotations

import re
from collections.abc import Callable
from typing import TypeAlias

FieldSpec: TypeAlias = tuple[str, str]
LogSummaryRenderer: TypeAlias = Callable[[str, str], str]


def parse_log_line(line: str) -> tuple[str, str] | None:
    match = re.match(r"^\[([^\]]+)\]\s+(.*)$", str(line or "").strip())
    if not match:
        return None
    return match.group(1), match.group(2)


def get_log_field(body: str, key: str) -> str:
    match = re.search(rf"(?:^|\s){re.escape(key)}=([^\s]+)", body)
    return match.group(1) if match else "-"


def _render_summary(timestamp: str, label: str, body: str, fields: tuple[FieldSpec, ...]) -> str:
    rendered_fields = " ".join(f"{name}={get_log_field(body, key)}" for name, key in fields)
    return f"{timestamp} {label} {rendered_fields}"


def _render_raw_message_untracked(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "raw_message_untracked", body, (("channel", "channel"), ("source", "source")))


def _render_raw_message(timestamp: str, body: str) -> str:
    return _render_summary(
        timestamp,
        "raw_message",
        body,
        (("channel", "channel"), ("source", "source"), ("bot", "bot"), ("content_len", "content_len")),
    )


def _render_message_received(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "message_received", body, (("channel", "chat"), ("content_len", "content_len")))


def _render_ignored_message(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "ignored_message", body, (("reason", "reason"), ("channel", "chat")))


def _render_history_poll_message(timestamp: str, body: str) -> str:
    return _render_summary(
        timestamp,
        "history_poll_message",
        body,
        (("channel", "channel"), ("content_len", "content_len")),
    )


def _render_history_poll_primed(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "history_poll_primed", body, (("channel", "channel"), ("messages", "messages")))


def _render_message_routed(timestamp: str, body: str) -> str:
    return _render_summary(
        timestamp,
        "message_routed",
        body,
        (("channel", "chat"), ("target", "target"), ("prefix", "prefix"), ("text_len", "text_len")),
    )


def _render_raw_interaction(timestamp: str, body: str) -> str:
    return _render_summary(
        timestamp,
        "raw_interaction",
        body,
        (("channel", "channel"), ("type", "type"), ("command", "command")),
    )


def _render_interaction_received(timestamp: str, body: str) -> str:
    return _render_summary(
        timestamp,
        "interaction_received",
        body,
        (("channel", "channel"), ("type", "type"), ("command", "command")),
    )


def _render_slash_event(timestamp: str, body: str) -> str:
    slash_event = body.split(" ", 1)[0]
    return _render_summary(
        timestamp,
        slash_event,
        body,
        (
            ("channel", "channel"),
            ("command", "command"),
            ("exit", "exit"),
            ("response", "response"),
            ("reason", "reason"),
        ),
    )


def _render_component_event(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "component_event", body, (("channel", "channel"), ("custom_id", "custom_id")))


def _render_busy_choice_event(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "busy_choice_event", body, (("reason", "reason"), ("target", "target")))


def _render_approval_persistent(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "approval_persistent", body, (("target", "target"), ("exit", "exit")))


def _render_input_choice_persistent(timestamp: str, body: str) -> str:
    return _render_summary(timestamp, "input_choice_persistent", body, (("target", "target"), ("exit", "exit")))


_PREFIX_RENDERERS: tuple[tuple[str, LogSummaryRenderer], ...] = (
    ("socket_message_create_untracked ", _render_raw_message_untracked),
    ("socket_message_create ", _render_raw_message),
    ("message_received ", _render_message_received),
    ("ignored_message ", _render_ignored_message),
    ("history_poll_message ", _render_history_poll_message),
    ("history_poll_primed ", _render_history_poll_primed),
    ("message ", _render_message_routed),
    ("socket_interaction_create ", _render_raw_interaction),
    ("interaction_received ", _render_interaction_received),
    ("component_interaction_", _render_component_event),
    ("busy_choice_", _render_busy_choice_event),
    ("approval_persistent", _render_approval_persistent),
    ("input_choice_persistent", _render_input_choice_persistent),
)


def summarize_discord_hook_log_line(line: str) -> str | None:
    parsed = parse_log_line(line)
    if not parsed:
        return None
    timestamp, body = parsed
    if body.startswith("slash_"):
        return _render_slash_event(timestamp, body)
    for prefix, renderer in _PREFIX_RENDERERS:
        if body.startswith(prefix):
            return renderer(timestamp, body)
    return None


def is_user_or_control_hook_summary(summary: str) -> bool:
    if " raw_message " in summary:
        return " bot=False " in summary or " bot=- " in summary
    if " raw_message_untracked " in summary:
        return True
    return any(
        marker in summary
        for marker in [
            " message_received ",
            " ignored_message ",
            " history_poll_message ",
            " message_routed ",
            " raw_interaction ",
            " interaction_received ",
            " slash_",
            " component_event ",
            " busy_choice_event ",
            " approval_persistent ",
            " input_choice_persistent ",
        ]
    )

"""File logging and hook-log summarization for the Discord Codex adapter."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

import codex_desktop_bridge as bridge
from codex_discord_log_summary import (
    get_log_field,
    is_user_or_control_hook_summary,
    parse_log_line,
    summarize_discord_hook_log_line,
)


__all__ = [
    "get_discord_log_markers",
    "get_log_field",
    "get_log_path",
    "get_recent_discord_hook_events",
    "is_user_or_control_hook_summary",
    "log_line",
    "parse_log_line",
    "summarize_discord_hook_log_line",
]


SCRIPT_DIR = Path(__file__).resolve().parent
LOG_PATH = SCRIPT_DIR / "codex_discord_bot.log"


def get_log_path() -> Path:
    value = os.environ.get("CODEX_DISCORD_LOG_PATH", "").strip()
    if value:
        return Path(value).expanduser()
    return LOG_PATH


def log_line(message: str) -> None:
    timestamp = time.strftime("%Y-%m-%d %H:%M:%S")
    line = f"[{timestamp}] {message}\n"
    log_path = get_log_path()
    try:
        bridge.rotate_single_backup_file(
            log_path,
            incoming_bytes=len(line.encode("utf-8")),
        )
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("a", encoding="utf-8") as handle:
            _ = handle.write(line)
    except OSError as exc:
        print(
            f"discord_log_write_failed path={log_path} "
            + f"error_type={type(exc).__name__} error={exc}",
            file=sys.stderr,
        )


def get_recent_discord_hook_events(
    *,
    limit: int = 8,
    max_lines: int = 1000,
    user_or_control_only: bool = False,
) -> list[str]:
    log_path = get_log_path()
    try:
        lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return []
    events: list[str] = []
    for line in lines[-max_lines:]:
        summary = summarize_discord_hook_log_line(line)
        if summary:
            if user_or_control_only and not is_user_or_control_hook_summary(summary):
                continue
            events.append(summary)
    return events[-limit:]


def get_discord_log_markers(*, max_lines: int = 2000) -> dict[str, str]:
    log_path = get_log_path()
    markers = {
        "last_ready_at": "-",
        "last_gateway_event_at": "-",
        "last_raw_interaction_at": "-",
        "last_interaction_at": "-",
        "last_component_at": "-",
        "last_user_or_control_hook_at": "-",
        "last_button_qa_at": "-",
        "last_button_qa_result": "-",
        "last_steering_button_at": "-",
        "last_steering_button_exit": "-",
        "last_steering_button_elapsed_sec": "-",
    }
    try:
        lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return markers
    for line in lines[-max_lines:]:
        parsed = parse_log_line(line)
        if not parsed:
            continue
        timestamp, body = parsed
        if body.startswith("ready "):
            markers["last_ready_at"] = timestamp
        if body.startswith(("socket_message_create ", "socket_message_create_untracked ", "socket_interaction_create ")):
            markers["last_gateway_event_at"] = timestamp
        if body.startswith("socket_interaction_create "):
            markers["last_raw_interaction_at"] = timestamp
        if body.startswith("interaction_received "):
            markers["last_interaction_at"] = timestamp
            if " type=component " in f" {body} ":
                markers["last_component_at"] = timestamp
        if body.startswith((
            "component_interaction_",
            "busy_choice_persistent_",
            "approval_persistent",
            "input_choice_persistent",
        )):
            markers["last_component_at"] = timestamp
        if body.startswith("button_qa_done "):
            markers["last_button_qa_at"] = timestamp
            markers["last_button_qa_result"] = get_log_field(body, "result")
        if body.startswith(("steer_now_done ", "busy_choice_persistent_steer_done ")):
            markers["last_steering_button_at"] = timestamp
            markers["last_steering_button_exit"] = get_log_field(body, "exit")
            markers["last_steering_button_elapsed_sec"] = get_log_field(body, "elapsed_sec")
        summary = summarize_discord_hook_log_line(line)
        if summary and is_user_or_control_hook_summary(summary):
            markers["last_user_or_control_hook_at"] = timestamp
    return markers

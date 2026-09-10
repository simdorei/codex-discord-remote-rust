from __future__ import annotations

from typing import Literal, TypeAlias

StreamRelayLineKind: TypeAlias = Literal[
    "commentary",
    "final",
    "failed",
    "transport_error",
    "timeout",
    "aborted",
    "ready",
    "waiting",
    "ignored",
    "content",
]


def classify_stream_relay_line(line: str) -> StreamRelayLineKind:
    if line.startswith("[commentary]"):
        return "commentary"
    if line.startswith("[final_answer]"):
        return "final"
    if line.startswith("[failed]"):
        return "failed"
    if line.startswith("[transport_error]"):
        return "transport_error"
    if line.startswith("[timeout]"):
        return "timeout"
    if line.startswith("[aborted]"):
        return "aborted"
    if line.startswith("[ready]"):
        return "ready"
    if line.startswith("[waiting_for_final_answer]") or line.startswith("Use Ctrl+C"):
        return "waiting"
    if is_ignored_stream_relay_line(line):
        return "ignored"
    return "content"


def is_ignored_stream_relay_line(line: str) -> bool:
    if line.startswith(("target_thread:", "title:", "ui_name:", "cwd:")):
        return True
    if line.startswith(("ui_activation:", "sent_to_window:", "[delivery_verified]")):
        return True
    if line.startswith(("[background_watch_started]", "[background_watch_already_running]")):
        return True
    return line.startswith("[wait_cancelled]")

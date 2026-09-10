from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from codex_bridge_state import JsonObject


ExtractMessageText = Callable[[JsonObject], str]
PrintLine = Callable[[str], None]
ReadNewSessionEvents = Callable[[Path, int], tuple[list[JsonObject], int]]
Sleep = Callable[[float], None]
TimeNow = Callable[[], float]


@dataclass(frozen=True, slots=True)
class TailDeps:
    read_new_session_events: ReadNewSessionEvents
    extract_message_text: ExtractMessageText
    time_now: TimeNow
    sleep: Sleep
    print_line: PrintLine


def tail_session_events(
    session_path: Path,
    *,
    only_new: bool,
    timeout: float,
    deps: TailDeps,
) -> None:
    start_offset = session_path.stat().st_size if only_new else 0
    deadline = deps.time_now() + timeout if timeout > 0 else None
    cursor = start_offset
    seen_agent_messages: set[str] = set()

    while True:
        events, cursor = deps.read_new_session_events(session_path, cursor)
        for event in events:
            _print_tail_event(event, seen_agent_messages, deps)

        if deadline is not None and deps.time_now() >= deadline:
            return
        deps.sleep(0.35)


def _print_tail_event(
    event: JsonObject,
    seen_agent_messages: set[str],
    deps: TailDeps,
) -> None:
    payload = event.get("payload")
    if not isinstance(payload, dict):
        return

    if event.get("type") == "event_msg" and payload.get("type") == "agent_message":
        _print_agent_message(payload, seen_agent_messages, deps.print_line)
        return

    if event.get("type") == "response_item" and payload.get("type") == "message":
        _print_response_message(payload, seen_agent_messages, deps)


def _print_agent_message(
    payload: JsonObject,
    seen_agent_messages: set[str],
    print_line: PrintLine,
) -> None:
    if str(payload.get("phase", "") or "") == "final_answer":
        return
    message = str(payload.get("message", "")).strip()
    if not message:
        return
    seen_agent_messages.add(message)
    print_line(f"[commentary] {message}")


def _print_response_message(
    payload: JsonObject,
    seen_agent_messages: set[str],
    deps: TailDeps,
) -> None:
    text = deps.extract_message_text(payload)
    role = payload.get("role", "?")
    phase = payload.get("phase", "")
    if not text:
        return
    if role == "assistant" and phase == "commentary" and text in seen_agent_messages:
        return
    deps.print_line(f"[{role}:{phase}]")
    deps.print_line(text)
    deps.print_line("")

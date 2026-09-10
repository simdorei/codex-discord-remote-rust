from __future__ import annotations

from collections.abc import Callable, Iterable, Iterator
from dataclasses import dataclass, replace
from pathlib import Path
import time
from typing import Protocol, TypeAlias

from codex_session_events import JsonEvent, JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]


class BusySessionDeps(Protocol):
    @property
    def iter_session_events(self) -> Callable[[Path], Iterator[JsonEvent]]: ...

    @property
    def time_now(self) -> Callable[[], float]: ...

    @property
    def get_orphan_task_started_grace_seconds(self) -> Callable[[], float]: ...

    @property
    def get_stale_busy_session_seconds(self) -> Callable[[], float]: ...

    @property
    def get_pending_interactive_state_from_session(self) -> Callable[[Path], str | None]: ...


@dataclass(frozen=True, slots=True)
class _BusySessionActivity:
    last_started: int = -1
    last_complete: int = -1
    last_final: int = -1
    last_aborted: int = -1
    last_activity: int = -1


def session_file_age_seconds(
    session_path: Path,
    *,
    now: float | None = None,
    time_now: Callable[[], float] = time.time,
) -> float | None:
    try:
        mtime = session_path.stat().st_mtime
    except OSError:
        return None
    current = time_now() if now is None else now
    return max(0.0, current - mtime)


def is_thread_busy(session_path: Path, *, deps: BusySessionDeps) -> bool:
    try:
        activity = _scan_busy_session_activity(deps.iter_session_events(session_path))
    except OSError:
        return False

    last_done = max(activity.last_complete, activity.last_final, activity.last_aborted)
    if activity.last_started <= last_done:
        return False

    age_seconds = session_file_age_seconds(session_path, time_now=deps.time_now)
    if activity.last_activity < activity.last_started:
        if age_seconds is not None and age_seconds >= deps.get_orphan_task_started_grace_seconds():
            if deps.get_pending_interactive_state_from_session(session_path):
                return True
            return False
    elif age_seconds is not None and age_seconds >= deps.get_stale_busy_session_seconds():
        if deps.get_pending_interactive_state_from_session(session_path):
            return True
        return False

    return True


def _scan_busy_session_activity(events: Iterable[JsonEvent]) -> _BusySessionActivity:
    activity = _BusySessionActivity()
    for index, event in enumerate(events):
        payload = _busy_event_payload(event)
        if payload is None:
            continue

        event_type = event.get("type")
        if event_type == "event_msg":
            activity = _record_event_msg_activity(activity, payload, index)
        elif event_type == "response_item":
            activity = _record_response_item_activity(activity, payload, index)
    return activity


def _busy_event_payload(event: JsonEvent) -> JsonObject | None:
    payload = event.get("payload") or {}
    if isinstance(payload, dict):
        return payload
    return None


def _record_event_msg_activity(activity: _BusySessionActivity, payload: JsonObject, index: int) -> _BusySessionActivity:
    event_type = payload.get("type")
    if event_type == "task_started":
        return replace(activity, last_started=index)
    if event_type == "task_complete":
        return replace(activity, last_complete=index)
    if event_type in {"turn_aborted", "task_aborted", "task_cancelled"}:
        return replace(activity, last_aborted=index)
    if event_type not in {"user_message", "agent_message"}:
        return activity
    if event_type == "agent_message" and payload.get("phase") == "final_answer":
        return replace(activity, last_activity=index, last_final=index)
    return replace(activity, last_activity=index)


def _record_response_item_activity(activity: _BusySessionActivity, payload: JsonObject, index: int) -> _BusySessionActivity:
    payload_type = payload.get("type")
    if payload_type not in {"message", "function_call", "custom_tool_call"}:
        return activity
    if payload_type != "message":
        return replace(activity, last_activity=index)

    role = payload.get("role")
    if role == "assistant" and payload.get("phase") == "final_answer":
        return replace(activity, last_activity=index, last_final=index)
    if role in {"user", "assistant"}:
        return replace(activity, last_activity=index)
    return activity

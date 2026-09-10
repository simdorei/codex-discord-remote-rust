from __future__ import annotations

from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias

import codex_desktop_bridge_busy_session as busy_session
from codex_session_events import JsonEvent, JsonValue
from codex_thread_models import ThreadInfo

JsonObject: TypeAlias = dict[str, JsonValue]
is_thread_busy = busy_session.is_thread_busy
session_file_age_seconds = busy_session.session_file_age_seconds


class BusyThread(Protocol):
    @property
    def id(self) -> str: ...

    @property
    def rollout_path(self) -> str: ...


class SidecarClient(Protocol):
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject: ...

    def close(self) -> None: ...


@dataclass(frozen=True, slots=True)
class BusyStateDeps:
    iter_session_events: Callable[[Path], Iterator[JsonEvent]]
    time_now: Callable[[], float]
    get_orphan_task_started_grace_seconds: Callable[[], float]
    get_stale_busy_session_seconds: Callable[[], float]
    get_pending_interactive_state_from_session: Callable[[Path], str | None]
    load_recent_threads: Callable[[int], list[ThreadInfo]]
    make_sidecar: Callable[[], SidecarClient]
    get_sidecar_thread_status_type: Callable[[JsonObject], str]
    ensure_thread_loaded_via_sidecar: Callable[[SidecarClient, str], JsonObject]


def get_busy_threads(*, limit: int = 50, deps: BusyStateDeps) -> list[ThreadInfo]:
    busy_threads: list[ThreadInfo] = []
    client: SidecarClient | None = None
    try:
        try:
            client = deps.make_sidecar()
        except Exception:  # noqa: BROAD_EXCEPT_OK - preserve transport fallback behavior.
            client = None
        for thread in deps.load_recent_threads(limit):
            session_path = Path(thread.rollout_path)
            if not session_path.exists():
                continue
            if client is not None:
                if get_thread_busy_state(thread, client=client, allow_resume=True, deps=deps) != "idle":
                    busy_threads.append(thread)
                continue
            if is_thread_busy(session_path, deps=deps):
                busy_threads.append(thread)
    finally:
        if client is not None:
            client.close()
    return busy_threads


def classify_thread_status(status_payload: JsonObject | None) -> str | None:
    if status_payload is None:
        return None
    status_type = str(status_payload.get("type") or "").strip()
    if not status_type:
        return None

    if status_type == "active":
        active_flags = set(_string_items(status_payload.get("activeFlags")))
        if "waitingOnUserInput" in active_flags:
            return "waiting-input"
        if "waitingOnApproval" in active_flags:
            return "waiting-approval"
        return "busy"
    if status_type in {"idle", "notLoaded"}:
        return None
    return "busy"


def get_thread_busy_state(
    thread: BusyThread,
    *,
    deps: BusyStateDeps,
    client: SidecarClient | None = None,
    allow_resume: bool = False,
) -> str:
    session_path = Path(thread.rollout_path)
    if not session_path.exists() or not is_thread_busy(session_path, deps=deps):
        return "idle"

    interactive_state = deps.get_pending_interactive_state_from_session(session_path)
    own_client = False
    if client is None:
        try:
            client = deps.make_sidecar()
            own_client = True
        except Exception:  # noqa: BROAD_EXCEPT_OK - preserve transport fallback behavior.
            client = None

    try:
        if client is not None:
            thread_payload = _get_thread_payload(client.read_thread(thread.id, include_turns=False))
            if deps.get_sidecar_thread_status_type(thread_payload) == "notLoaded" and allow_resume:
                thread_payload = deps.ensure_thread_loaded_via_sidecar(client, thread.id)
            status_type = deps.get_sidecar_thread_status_type(thread_payload)
            status_payload = thread_payload.get("status")
            classified = classify_thread_status(status_payload if isinstance(status_payload, dict) else None)
            if classified:
                return classified
            if status_type in {"idle", "notLoaded"} and not interactive_state:
                return "idle"
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - preserve transport fallback behavior.
        _ = exc
    finally:
        if own_client and client is not None:
            client.close()

    if interactive_state:
        return interactive_state

    return "busy"


def describe_thread_busy_state(state: str) -> str:
    if state == "waiting-input":
        return (
            "The selected thread is waiting on a follow-up choice or input in Codex Desktop. "
            "Open the thread in the app and respond there first."
        )
    if state == "waiting-approval":
        return (
            "The selected thread is waiting on an approval prompt in Codex Desktop. "
            "Open the thread in the app and approve, reject, or cancel it first."
        )
    return (
        "The selected thread is still busy. This often means the same Codex thread is currently active "
        "or another task is still running. Wait, switch to another thread, or pass --force-while-busy."
    )


def _get_thread_payload(response: JsonObject) -> JsonObject:
    thread_payload = response.get("thread")
    if isinstance(thread_payload, dict):
        return thread_payload
    return {}


def _string_items(value: JsonValue) -> Iterator[str]:
    if not isinstance(value, list):
        return
    for item in value:
        text = str(item).strip()
        if text:
            yield text

from __future__ import annotations

import time
from collections.abc import Callable
from typing import Final, Protocol

from codex_app_server_transport_replies import (
    CodexAppServerTransportError,
    JsonMapping,
    JsonObject,
)


class ThreadLoaderClient(Protocol):
    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def resume_thread(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 10.0,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def is_thread_subscribed(self, thread_id: str) -> bool: ...


THREAD_LOAD_TIMEOUT_SEC: Final = 40.0


def get_in_progress_turn_id(thread_payload: JsonMapping) -> str | None:
    turns = thread_payload.get("turns") or []
    if not isinstance(turns, list):
        return None
    for turn in reversed(turns):
        if not isinstance(turn, dict):
            continue
        turn_id = str(turn.get("id") or "").strip()
        status = str(turn.get("status") or "").strip()
        if turn_id and status == "inProgress":
            return turn_id
    return None


def extract_thread_id(params: JsonMapping) -> str:
    for key in ("threadId", "conversationId"):
        value = str(params.get(key) or "").strip()
        if value:
            return value
    thread = params.get("thread")
    if isinstance(thread, dict):
        return str(thread.get("id") or "").strip()
    turn = params.get("turn")
    if isinstance(turn, dict):
        return str(turn.get("threadId") or turn.get("conversationId") or "").strip()
    return ""


def extract_turn_id(params: JsonMapping) -> str:
    for key in ("turnId", "id"):
        value = str(params.get(key) or "").strip()
        if value:
            return value
    turn = params.get("turn")
    if isinstance(turn, dict):
        return str(turn.get("id") or "").strip()
    return ""


def get_thread_status_type(thread_payload: JsonMapping) -> str:
    status = thread_payload.get("status") or {}
    if isinstance(status, dict):
        return str(status.get("type") or "").strip()
    return ""


def ensure_thread_loaded(
    client: ThreadLoaderClient,
    thread_id: str,
    *,
    timeout_sec: float = THREAD_LOAD_TIMEOUT_SEC,
    monotonic_func: Callable[[], float] = time.monotonic,
    expected_generation: int | None = None,
) -> JsonObject:
    deadline = monotonic_func() + max(timeout_sec, 0.0)
    if expected_generation is None:
        thread_payload = client.read_thread(thread_id, include_turns=False).get("thread")
    else:
        thread_payload = client.read_thread(
            thread_id,
            include_turns=False,
            expected_generation=expected_generation,
        ).get("thread")
    if not isinstance(thread_payload, dict):
        return {}
    if get_thread_status_type(thread_payload) != "notLoaded" and client.is_thread_subscribed(thread_id):
        return thread_payload
    remaining = max(0.0, deadline - monotonic_func())
    if remaining <= 0:
        raise TimeoutError("Timed out waiting for app-server response to thread/resume.")
    if expected_generation is None:
        resumed = client.resume_thread(thread_id, timeout_sec=remaining)
    else:
        resumed = client.resume_thread(
            thread_id,
            timeout_sec=remaining,
            expected_generation=expected_generation,
        )
    resumed_thread = resumed.get("thread") or {}
    if not isinstance(resumed_thread, dict):
        raise CodexAppServerTransportError("thread/resume did not return a thread payload.")
    return resumed_thread


def result_turn_id(result: JsonMapping, fallback: str | None = None) -> str | None:
    turn = result.get("turn")
    if not isinstance(turn, dict):
        return fallback
    return str(turn.get("id") or fallback or "").strip() or fallback

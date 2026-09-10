from __future__ import annotations

from enum import StrEnum
from threading import Event, Lock
from typing import Final


DEFAULT_OPEN_WAIT_SECONDS: Final = 30.0


class GateMode(StrEnum):
    OPEN = "open"
    HOLD = "hold"
    DISCARD = "discard"


class ProSessionMirrorGateTimeoutError(RuntimeError):
    def __init__(self, target_thread_id: str | None) -> None:
        self.target_thread_id = target_thread_id
        super().__init__(
            "Timed out waiting for rejected Pro output to be discarded before delivery."
        )


_LOCK = Lock()
_MODES: dict[str, GateMode] = {}
_DISCARD_SIZES: dict[str, int] = {}
_OPEN_EVENTS: dict[str, Event] = {}


def hold(target_thread_id: str | None) -> None:
    key = _target_key(target_thread_id)
    if key is None:
        return
    with _LOCK:
        _MODES[key] = GateMode.HOLD
        _DISCARD_SIZES.pop(key, None)
        _closed_event(key).clear()


def approve(target_thread_id: str | None) -> None:
    _clear(target_thread_id)


def reject(target_thread_id: str | None) -> None:
    key = _target_key(target_thread_id)
    if key is None:
        return
    with _LOCK:
        _MODES[key] = GateMode.DISCARD
        _DISCARD_SIZES.pop(key, None)
        _closed_event(key).clear()


def mode(target_thread_id: str | None) -> GateMode:
    key = _target_key(target_thread_id)
    if key is None:
        return GateMode.OPEN
    with _LOCK:
        return _MODES.get(key, GateMode.OPEN)


def wait_until_open(
    target_thread_id: str | None,
    timeout_seconds: float = DEFAULT_OPEN_WAIT_SECONDS,
) -> bool:
    key = _target_key(target_thread_id)
    if key is None:
        return True
    with _LOCK:
        open_event = _OPEN_EVENTS.get(key)
    if open_event is None:
        return True
    return open_event.wait(timeout_seconds)


def discard_size_is_stable(target_thread_id: str | None, rollout_size: int) -> bool:
    key = _target_key(target_thread_id)
    if key is None:
        return False
    with _LOCK:
        if _MODES.get(key) is not GateMode.DISCARD:
            return False
        previous_size = _DISCARD_SIZES.get(key)
        _DISCARD_SIZES[key] = rollout_size
        return previous_size == rollout_size


def finish_discard(target_thread_id: str | None) -> None:
    _clear(target_thread_id)


def reset_for_tests() -> None:
    with _LOCK:
        for open_event in _OPEN_EVENTS.values():
            open_event.set()
        _MODES.clear()
        _DISCARD_SIZES.clear()
        _OPEN_EVENTS.clear()


def _clear(target_thread_id: str | None) -> None:
    key = _target_key(target_thread_id)
    if key is None:
        return
    with _LOCK:
        _MODES.pop(key, None)
        _DISCARD_SIZES.pop(key, None)
        open_event = _OPEN_EVENTS.pop(key, None)
        if open_event is not None:
            open_event.set()


def _closed_event(key: str) -> Event:
    open_event = _OPEN_EVENTS.get(key)
    if open_event is None:
        open_event = Event()
        _OPEN_EVENTS[key] = open_event
    return open_event


def _target_key(target_thread_id: str | None) -> str | None:
    key = (target_thread_id or "").strip()
    return key or None

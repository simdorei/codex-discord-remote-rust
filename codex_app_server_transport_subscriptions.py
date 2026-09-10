from __future__ import annotations

import threading
import time
from collections.abc import Callable, Generator
from contextlib import contextmanager
from dataclasses import dataclass
from enum import StrEnum, unique
from typing import Protocol, final

from codex_app_server_transport_goal import (
    GoalPresent,
    GoalTransportError,
    ThreadGoalLookup,
    is_terminal_goal_status,
)
from codex_app_server_transport_replies import (
    CodexAppServerTransportError,
    JsonMapping,
    JsonObject,
)

LogFunc = Callable[[str], None]
MonotonicFunc = Callable[[], float]
_LOCK_STRIPES = 64
_MAX_RETRY_SECONDS = 60.0


class ThreadSubscriptionReleaseClient(Protocol):
    def cancel_pending_server_requests(self, thread_id: str) -> int: ...

    def has_active_turn_or_raise(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> bool: ...

    def get_pending_server_requests(self, thread_id: str | None = None) -> list[JsonObject]: ...

    def get_thread_goal_lookup(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 3.0,
        expected_generation: int | None = None,
    ) -> ThreadGoalLookup: ...

    def request(
        self,
        method: str,
        params: JsonMapping | None = None,
        *,
        timeout_sec: float = 10.0,
        expected_generation: int | None = None,
    ) -> JsonObject: ...


@unique
class ThreadReleaseStatus(StrEnum):
    RELEASED = "released"
    ALREADY_RELEASED = "already_released"
    ACTIVE_TURN = "active_turn"
    PENDING_REQUEST = "pending_request"
    ACTIVE_GOAL = "active_goal"
    RETRY_DEFERRED = "retry_deferred"
    FAILED = "failed"


@dataclass(frozen=True, slots=True)
class ThreadReleaseOutcome:
    status: ThreadReleaseStatus
    error_message: str = ""
    retry_after_seconds: float = 0.0

    @property
    def released(self) -> bool:
        return self.status in {
            ThreadReleaseStatus.RELEASED,
            ThreadReleaseStatus.ALREADY_RELEASED,
        }


@dataclass(frozen=True, slots=True)
class _ReleaseRetry:
    attempts: int
    retry_at: float


@final
class ThreadSubscriptionCoordinator:
    def __init__(self, *, monotonic_func: MonotonicFunc = time.monotonic) -> None:
        self._monotonic_func: MonotonicFunc = monotonic_func
        self._state_lock: threading.RLock = threading.RLock()
        self._lifecycle_locks: tuple[threading.RLock, ...] = tuple(
            threading.RLock() for _ in range(_LOCK_STRIPES)
        )
        self._subscribed: set[str] = set()
        self._release_retries: dict[str, _ReleaseRetry] = {}

    @contextmanager
    def lifecycle_lock(self, thread_id: str) -> Generator[None, None, None]:
        lock = self._lifecycle_locks[hash(thread_id) % len(self._lifecycle_locks)]
        with lock:
            yield

    def clear(self) -> None:
        with self._state_lock:
            self._subscribed.clear()
            self._release_retries.clear()

    def is_subscribed(self, thread_id: str) -> bool:
        with self._state_lock:
            return thread_id in self._subscribed

    def has_subscriptions(self) -> bool:
        with self._state_lock:
            return bool(self._subscribed)

    def subscribed_thread_ids(self) -> tuple[str, ...]:
        with self._state_lock:
            return tuple(sorted(self._subscribed))

    def mark_subscribed(self, thread_id: str) -> None:
        with self._state_lock:
            self._subscribed.add(thread_id)
            _ = self._release_retries.pop(thread_id, None)

    def note_thread_activity(self, thread_id: str) -> None:
        with self._state_lock:
            _ = self._release_retries.pop(thread_id, None)

    def release_if_terminal(
        self,
        client: ThreadSubscriptionReleaseClient,
        thread_id: str,
        *,
        expected_generation: int | None = None,
        log: LogFunc,
    ) -> ThreadReleaseOutcome:
        with self.lifecycle_lock(thread_id):
            return self._release_locked(
                client,
                thread_id,
                expected_generation=expected_generation,
                log=log,
            )

    def _release_locked(
        self,
        client: ThreadSubscriptionReleaseClient,
        thread_id: str,
        *,
        expected_generation: int | None,
        log: LogFunc,
    ) -> ThreadReleaseOutcome:
        if not self.is_subscribed(thread_id):
            return ThreadReleaseOutcome(ThreadReleaseStatus.ALREADY_RELEASED)
        retry = self._retry_wait(thread_id)
        if retry > 0:
            return ThreadReleaseOutcome(
                ThreadReleaseStatus.RETRY_DEFERRED,
                retry_after_seconds=retry,
            )
        try:
            if expected_generation is None:
                has_active_turn = client.has_active_turn_or_raise(thread_id)
            else:
                has_active_turn = client.has_active_turn_or_raise(
                    thread_id,
                    expected_generation=expected_generation,
                )
            if has_active_turn:
                return ThreadReleaseOutcome(ThreadReleaseStatus.ACTIVE_TURN)
            if expected_generation is None:
                goal = client.get_thread_goal_lookup(thread_id)
            else:
                goal = client.get_thread_goal_lookup(
                    thread_id,
                    expected_generation=expected_generation,
                )
            if isinstance(goal, GoalTransportError):
                return self._record_failure(thread_id, goal.message, "GoalTransportError", log)
            if isinstance(goal, GoalPresent) and not is_terminal_goal_status(goal.status):
                return ThreadReleaseOutcome(ThreadReleaseStatus.ACTIVE_GOAL)
            pending_requests = client.get_pending_server_requests(thread_id)
            if pending_requests:
                _ = client.cancel_pending_server_requests(thread_id)
            if expected_generation is None:
                _ = client.request(
                    "thread/unsubscribe",
                    {"threadId": thread_id},
                    timeout_sec=8.0,
                )
            else:
                _ = client.request(
                    "thread/unsubscribe",
                    {"threadId": thread_id},
                    timeout_sec=8.0,
                    expected_generation=expected_generation,
                )
        except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
            return self._record_failure(thread_id, str(exc), type(exc).__name__, log)

        with self._state_lock:
            self._subscribed.discard(thread_id)
            _ = self._release_retries.pop(thread_id, None)
        log(f"app_server_thread_unsubscribed target={thread_id}")
        return ThreadReleaseOutcome(ThreadReleaseStatus.RELEASED)

    def _retry_wait(self, thread_id: str) -> float:
        now = self._monotonic_func()
        with self._state_lock:
            retry = self._release_retries.get(thread_id)
        if retry is None:
            return 0.0
        return max(0.0, retry.retry_at - now)

    def _record_failure(
        self,
        thread_id: str,
        message: str,
        error_type: str,
        log: LogFunc,
    ) -> ThreadReleaseOutcome:
        now = self._monotonic_func()
        with self._state_lock:
            previous = self._release_retries.get(thread_id)
            attempts = 1 if previous is None else previous.attempts + 1
            delay = min(_MAX_RETRY_SECONDS, 2.0 ** min(attempts - 1, 6))
            self._release_retries[thread_id] = _ReleaseRetry(attempts, now + delay)
        safe_message = message.replace("\r", " ").replace("\n", " ")[:300]
        log(
            f"app_server_thread_unsubscribe_failed target={thread_id} "
            + f"error_type={error_type} error={safe_message} retry_after_sec={delay:g}"
        )
        return ThreadReleaseOutcome(
            ThreadReleaseStatus.FAILED,
            error_message=message,
            retry_after_seconds=delay,
        )

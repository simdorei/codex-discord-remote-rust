from __future__ import annotations

import threading
from collections.abc import Callable
from typing import Final, final

from codex_app_server_transport_lifecycle import (
    AppServerGenerationMismatch,
    ChildCleanupRecycleOutcome,
    ChildCleanupRecycleStatus,
)
from codex_app_server_transport_replies import CodexAppServerTransportError


RetryFunc = Callable[[int], ChildCleanupRecycleOutcome]
RestartRetryFunc = Callable[[int], bool]
LogFunc = Callable[[str], None]
_INITIAL_RETRY_SECONDS: Final = 0.25
_MAX_RETRY_SECONDS: Final = 5.0
_SETTLED_STATUSES: Final = frozenset(
    (
        ChildCleanupRecycleStatus.RECYCLED,
        ChildCleanupRecycleStatus.NO_CLEANUP_DEBT,
    )
)


@final
class ChildCleanupRetryCoordinator:
    """Run one coalesced retry loop while generation cleanup remains deferred."""

    def __init__(self, *, retry: RetryFunc, log: LogFunc) -> None:
        self._retry = retry
        self._log = log
        self._lock = threading.Lock()
        self._wakeup = threading.Event()
        self._requested_generation: int | None = None
        self._thread: threading.Thread | None = None

    def schedule(self, generation: int) -> None:
        with self._lock:
            self._requested_generation = generation
            thread = self._thread
            if thread is None or not thread.is_alive():
                thread = threading.Thread(
                    target=self._run,
                    daemon=True,
                    name="codex-app-server-cleanup-retry",
                )
                self._thread = thread
                thread.start()
        self._wakeup.set()

    def wake(self) -> None:
        self._wakeup.set()

    def stop(self) -> None:
        with self._lock:
            self._requested_generation = None
        self._wakeup.set()

    def _run(self) -> None:
        delay = _INITIAL_RETRY_SECONDS
        while True:
            with self._lock:
                generation = self._requested_generation
                if generation is None:
                    self._thread = None
                    return
            try:
                outcome = self._retry(generation)
            except AppServerGenerationMismatch:
                self._settle(generation)
                delay = _INITIAL_RETRY_SECONDS
                continue
            except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
                self._log(
                    "app_server_child_cleanup_retry_failed "
                    + f"generation={generation} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
            except Exception as exc:  # noqa: BLE001 - daemon boundary must not fail silently.
                self._log(
                    "app_server_child_cleanup_retry_unexpected_failure "
                    + f"generation={generation} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
            else:
                if outcome.status in _SETTLED_STATUSES:
                    self._settle(generation)
                    delay = _INITIAL_RETRY_SECONDS
                    continue
            _ = self._wakeup.wait(timeout=delay)
            self._wakeup.clear()
            delay = min(delay * 2.0, _MAX_RETRY_SECONDS)

    def _settle(self, generation: int) -> None:
        with self._lock:
            if self._requested_generation == generation:
                self._requested_generation = None


@final
class RestartPendingRetryCoordinator:
    """Retry a quarantined generation restart until it succeeds or is replaced."""

    def __init__(self, *, retry: RestartRetryFunc, log: LogFunc) -> None:
        self._retry = retry
        self._log = log
        self._lock = threading.Lock()
        self._wakeup = threading.Event()
        self._requested_generation: int | None = None
        self._thread: threading.Thread | None = None

    def schedule(self, generation: int) -> None:
        with self._lock:
            self._requested_generation = generation
            thread = self._thread
            if thread is None or not thread.is_alive():
                thread = threading.Thread(
                    target=self._run,
                    daemon=True,
                    name="codex-app-server-restart-pending-retry",
                )
                self._thread = thread
                thread.start()
        self._wakeup.set()

    def wake(self) -> None:
        self._wakeup.set()

    def stop(self) -> None:
        with self._lock:
            self._requested_generation = None
        self._wakeup.set()

    def _run(self) -> None:
        delay = _INITIAL_RETRY_SECONDS
        while True:
            with self._lock:
                generation = self._requested_generation
                if generation is None:
                    self._thread = None
                    return
            try:
                settled = self._retry(generation)
            except AppServerGenerationMismatch:
                settled = True
            except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
                self._log(
                    "app_server_restart_pending_retry_failed "
                    + f"generation={generation} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
                settled = False
            except Exception as exc:  # noqa: BLE001 - daemon boundary must not fail silently.
                self._log(
                    "app_server_restart_pending_retry_unexpected_failure "
                    + f"generation={generation} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
                settled = False
            if settled:
                self._settle(generation)
                delay = _INITIAL_RETRY_SECONDS
                continue
            _ = self._wakeup.wait(timeout=delay)
            self._wakeup.clear()
            delay = min(delay * 2.0, _MAX_RETRY_SECONDS)

    def _settle(self, generation: int) -> None:
        with self._lock:
            if self._requested_generation == generation:
                self._requested_generation = None

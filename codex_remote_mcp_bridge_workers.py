from __future__ import annotations

import queue
import threading
from collections.abc import Callable
from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass
from time import monotonic
from typing import final

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    BridgeResult,
    GatewayCommand,
    OperationErrorResult,
)


@dataclass(frozen=True, slots=True)
class WorkerCompletion:
    generation: int
    result: BridgeResult


@final
class BridgeCommandWorkers:
    """Runs a bounded number of local commands away from the socket loop."""

    def __init__(
        self,
        dispatcher: LocalProjectDispatcher,
        *,
        log: Callable[[str], None],
        max_workers: int = 4,
        max_pending: int = 16,
    ) -> None:
        self._dispatcher = dispatcher
        self._log = log
        self._executor = ThreadPoolExecutor(
            max_workers=max_workers,
            thread_name_prefix="codex-remote-mcp-worker",
        )
        self._capacity = threading.BoundedSemaphore(max_pending)
        self._completed: queue.Queue[WorkerCompletion] = queue.Queue()
        self._generation_lock = threading.Lock()
        self._generation = 0
        self._connection_cancel_event = threading.Event()
        self._close_lock = threading.Lock()
        self._idle = threading.Condition(self._close_lock)
        self._accepting = True
        self._in_flight = 0
        self._closed = False

    def begin_connection(self) -> int:
        with self._generation_lock:
            self._connection_cancel_event.set()
            self._connection_cancel_event = threading.Event()
            self._generation += 1
            generation = self._generation
        self._dispatcher.begin_connection(generation)
        return generation

    def submit(
        self,
        generation: int,
        command: GatewayCommand,
    ) -> OperationErrorResult | None:
        with self._idle:
            if self._closed or not self._accepting:
                self._log("remote_mcp_bridge_command_rejected reason=closing")
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code="bridge_closing",
                    message="The local bridge is closing and cannot accept new work.",
                )
            if not self._capacity.acquire(blocking=False):
                self._log("remote_mcp_bridge_command_rejected reason=capacity")
                return OperationErrorResult(
                    request_id=command.request_id,
                    error_code="bridge_busy",
                    message="The local bridge is busy. Retry this request shortly.",
                )
            self._in_flight += 1
            future = self._executor.submit(
                self._dispatcher.execute,
                command,
                connection_generation=generation,
                cancel_event=self._cancel_event_for(generation),
            )
        _ = future.add_done_callback(
            lambda completed: self._finish(generation, command, completed)
        )
        return None

    def drain(self, generation: int) -> tuple[BridgeResult, ...]:
        current: list[BridgeResult] = []
        while True:
            try:
                completed = self._completed.get_nowait()
            except queue.Empty:
                return tuple(current)
            if completed.generation == generation:
                current.append(completed.result)
            else:
                self._log("remote_mcp_bridge_result_discarded reason=stale_connection")

    def close(self) -> None:
        with self._idle:
            if self._closed:
                return
            self._closed = True
            self._accepting = False
            with self._generation_lock:
                self._connection_cancel_event.set()
        self._executor.shutdown(wait=False, cancel_futures=True)

    def prepare_restart(self, timeout_seconds: float) -> bool:
        deadline = monotonic() + timeout_seconds
        with self._idle:
            self._accepting = False
            ready = self._idle.wait_for(
                lambda: self._in_flight == 0,
                timeout=max(0.0, deadline - monotonic()),
            )
            if not ready:
                self._accepting = True
            return ready

    def cancel_restart_preparation(self) -> None:
        with self._idle:
            if not self._closed:
                self._accepting = True
                self._idle.notify_all()

    def _cancel_event_for(self, generation: int) -> threading.Event:
        with self._generation_lock:
            if generation == self._generation:
                return self._connection_cancel_event
        stale = threading.Event()
        stale.set()
        return stale

    def _finish(
        self,
        generation: int,
        command: GatewayCommand,
        future: Future[BridgeResult],
    ) -> None:
        try:
            result = future.result()
        except Exception as exc:  # noqa: BLE001  # noqa: BROAD_EXCEPT_OK
            self._log(
                "remote_mcp_bridge_command_failed " + f"error={type(exc).__name__}"
            )
            result = OperationErrorResult(
                request_id=command.request_id,
                error_code="internal_error",
                message="The local bridge command failed unexpectedly.",
            )
        finally:
            self._capacity.release()
        self._completed.put(WorkerCompletion(generation, result))
        with self._idle:
            self._in_flight -= 1
            self._idle.notify_all()


__all__ = ["BridgeCommandWorkers"]

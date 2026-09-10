from __future__ import annotations

import json
import threading
import time
import uuid
from collections.abc import Generator
from contextlib import AbstractContextManager, contextmanager
from pathlib import Path
from typing import Callable, IO

from codex_plugin_runtime_fingerprint import (
    PluginRuntimeFingerprintError,
    capture_required_plugin_fingerprint,
)

from codex_app_server_stderr_log import (
    APP_SERVER_STDERR_LOG_NAME,
    AppServerStderrRecorder,
)
from codex_app_server_transport_children import ChildLifecycleSnapshot, ChildLifecycleTracker
from codex_app_server_transport_messages import classify_app_server_transport_line
from codex_app_server_transport_lifecycle import (
    AppServerGenerationMismatch,
    AppServerGenerationQuarantinedError,
    AppServerLifecycleSnapshot,
    ChildCleanupRecycleOutcome,
    ChildCleanupRecycleStatus,
)
from codex_app_server_transport_process import (
    ResidentProcess,
    close_resident_app_server_process,
    has_resident_app_server_stdio,
    start_resident_app_server_process,
)
import codex_desktop_bridge_sidecar_resolver as bridge_resolver
from codex_app_server_transport_pending import PendingRequestState
from codex_app_server_transport_subscriptions import ThreadSubscriptionCoordinator
from codex_app_server_transport_goal import ThreadGoalUpdate
from codex_app_server_transport_replies import (
    CodexAppServerTransportError,
    JsonMapping,
    JsonObject,
    JsonValue,
    extract_response_result,
)
from codex_app_server_transport_retry import (
    ChildCleanupRetryCoordinator,
    RestartPendingRetryCoordinator,
)
from codex_app_server_transport_threads import get_thread_status_type
from codex_app_server_transport_turn_outcomes import (
    TurnCompletion,
    TurnCompletionFound,
    TurnCompletionObservation,
    TurnCompletionPending,
    TurnCompletionTransportError,
)


LogFunc = Callable[[str], None]
MonotonicFunc = Callable[[], float]
WallTimeFunc = Callable[[], float]
ExternalWorkGuard = Callable[[], bool]
GenerationSeedFunc = Callable[[], int]
PluginRuntimeFingerprintReader = Callable[[], str]
_decode_json_value: Callable[[str], JsonValue] = json.loads
_MAX_FRESH_READ_RETRY_SECONDS = 25.0
_AMBIGUOUS_TURN_START_GRACE_SECONDS = 25.0
_READ_TIMEOUT_DEGRADED_THRESHOLD = 2
_MAX_TIMED_OUT_THREAD_READS = 64


def _new_generation_seed() -> int:
    return (uuid.uuid4().int & ((1 << 63) - 1)) or 1


class ResidentCodexAppServerTransport:
    def __init__(
        self,
        *,
        executable_resolver: Callable[[], str] | None = None,
        log_func: LogFunc | None = None,
        monotonic_func: MonotonicFunc = time.monotonic,
        wall_time_func: WallTimeFunc = time.time,
        generation_seed_func: GenerationSeedFunc = _new_generation_seed,
        plugin_runtime_fingerprint_reader: PluginRuntimeFingerprintReader = (
            capture_required_plugin_fingerprint
        ),
        stderr_log_path: Path | None = None,
    ) -> None:
        self.log_func: LogFunc | None = log_func
        self.executable_resolver: Callable[[], str] = (
            executable_resolver
            if executable_resolver is not None
            else lambda: bridge_resolver.resolve_codex_app_server_executable(log_func=self._log)
        )
        self.monotonic_func: MonotonicFunc = monotonic_func
        self.wall_time_func: WallTimeFunc = wall_time_func
        self.plugin_runtime_fingerprint_reader: PluginRuntimeFingerprintReader = (
            plugin_runtime_fingerprint_reader
        )
        self.process: ResidentProcess | None = None
        self._stdout_thread: threading.Thread | None = None
        self._stderr_recorder: AppServerStderrRecorder | None = None
        self._stderr_log_path: Path = (
            stderr_log_path
            if stderr_log_path is not None
            else Path(__file__).resolve().parent / APP_SERVER_STDERR_LOG_NAME
        )
        self._lock: threading.RLock = threading.RLock()
        self._write_lock: threading.Lock = threading.Lock()
        self._request_lock: threading.Lock = threading.Lock()
        self._condition: threading.Condition = threading.Condition(self._lock)
        self._responses: dict[str, JsonObject] = {}
        self._active_request_id: str | None = None
        self._pending: PendingRequestState = PendingRequestState()
        self._children: ChildLifecycleTracker = ChildLifecycleTracker()
        self._subscriptions: ThreadSubscriptionCoordinator = ThreadSubscriptionCoordinator(
            monotonic_func=monotonic_func
        )
        self._recycle_lock: threading.Lock = threading.Lock()
        self._external_work_guard: ExternalWorkGuard | None = None
        self._active_deliveries: int = 0
        self._delivery_local: threading.local = threading.local()
        self._draining: bool = False
        self._closing: bool = False
        self._quarantined_generation: int | None = None
        self._restart_pending: bool = False
        self._ambiguous_turn_start_thread_id: str | None = None
        self._ambiguous_turn_start_request_id: str | None = None
        self._ambiguous_turn_start_deadline: float | None = None
        self._timed_out_thread_reads: dict[str, tuple[str, str]] = {}
        self._consecutive_read_timeouts: int = 0
        self._closed_error: str | None = None
        self._initialized: bool = False
        self._started_at: float = 0.0
        self._generation: int = 0
        self._generation_seed: int = max(1, int(generation_seed_func()))
        self._accepting_since: float | None = None
        self._plugin_runtime_fingerprint: str | None = None
        self._plugin_runtime_error: str | None = None
        self._cleanup_retry: ChildCleanupRetryCoordinator = ChildCleanupRetryCoordinator(
            retry=self._retry_child_cleanup_once,
            log=self._log,
        )
        self._restart_retry: RestartPendingRetryCoordinator = RestartPendingRetryCoordinator(
            retry=self._retry_restart_pending_once,
            log=self._log,
        )

    def _log(self, text: str) -> None:
        if self.log_func is not None:
            self.log_func(text)

    def start(self) -> None:
        _ = self._request_lock.acquire()
        try:
            self._start_with_request_slot_acquired()
        finally:
            self._request_lock.release()

    def _start_with_request_slot_acquired(self) -> None:
        with self._lock:
            self._closing = False
            if self._is_running() and self._initialized:
                if self._draining or self._restart_pending:
                    raise CodexAppServerTransportError("Resident Codex app-server is draining.")
                return
            self.close_locked()
            executable = self.executable_resolver()
            plugin_fingerprint, plugin_error = self._capture_plugin_runtime_fingerprint()
            self.process = start_resident_app_server_process(executable)
            if not has_resident_app_server_stdio(self.process):
                self.close_locked()
                raise CodexAppServerTransportError("Resident Codex app-server stdio is unavailable.")

            self._stderr_recorder = AppServerStderrRecorder(
                self.process,
                self._stderr_log_path,
                log=self._log,
            )
            self._stderr_recorder.start()
            self._responses.clear()
            self._active_request_id = None
            self._pending.clear()
            self._closed_error = None
            self._initialized = False
            self._started_at = time.time()
            process = self.process
            self._stdout_thread = threading.Thread(
                target=lambda: self._drain_stdout(process),
                daemon=True,
            )
            self._stdout_thread.start()

        _ = self._request_started(
            "initialize",
            {
                "clientInfo": {
                    "name": "codex-discord-remote",
                    "title": "Codex Discord Remote",
                    "version": "1.0",
                },
                "capabilities": {"experimentalApi": True},
            },
            timeout_sec=8.0,
            _request_slot_acquired=True,
        )
        self.notify("initialized", {})
        with self._lock:
            self._initialized = True
            self._generation = (
                self._generation_seed if self._generation == 0 else self._generation + 1
            )
            self._accepting_since = self.wall_time_func()
            self._plugin_runtime_fingerprint = plugin_fingerprint
            self._plugin_runtime_error = plugin_error
            self._children.reset(self._generation)
            self._draining = False
            self._quarantined_generation = None
            self._restart_pending = False
            self._ambiguous_turn_start_thread_id = None
            self._ambiguous_turn_start_request_id = None
            self._ambiguous_turn_start_deadline = None
            self._clear_timed_out_thread_reads()
        self._log(f"app_server_transport_started executable={executable}")

    def _capture_plugin_runtime_fingerprint(self) -> tuple[str | None, str | None]:
        try:
            return self.plugin_runtime_fingerprint_reader(), None
        except PluginRuntimeFingerprintError as exc:
            error = str(exc)
            self._log(
                "app_server_plugin_runtime_fingerprint_failed " + f"error={error}"
            )
            return None, error

    def _is_running(self) -> bool:
        return self.process is not None and self.process.poll() is None

    def is_running(self) -> bool:
        with self._lock:
            return self._is_running()

    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot:
        with self._lock:
            quarantined = self._quarantined_generation == self._generation
            healthy = (
                self._initialized
                and self._is_running()
                and not self._draining
                and not quarantined
                and not self._restart_pending
            )
            return AppServerLifecycleSnapshot(
                generation=self._generation,
                healthy=healthy,
                accepting_since=self._accepting_since if healthy else None,
                quarantined=quarantined,
                restart_pending=self._restart_pending,
                read_degraded=(
                    self._consecutive_read_timeouts >= _READ_TIMEOUT_DEGRADED_THRESHOLD
                ),
                consecutive_read_timeouts=self._consecutive_read_timeouts,
                plugin_runtime_fingerprint=self._plugin_runtime_fingerprint,
                plugin_runtime_error=self._plugin_runtime_error,
            )

    def _raise_for_generation_mismatch(self, expected_generation: int) -> None:
        snapshot = self.lifecycle_snapshot()
        if not snapshot.healthy or snapshot.generation != expected_generation:
            raise AppServerGenerationMismatch(
                expected_generation=expected_generation,
                actual_generation=snapshot.generation,
                healthy=snapshot.healthy,
            )

    def _drain_stdout(self, process: ResidentProcess | None) -> None:
        try:
            stdout: IO[str] | None = process.stdout if process is not None else None
            if stdout is None:
                return
            while True:
                raw_line = stdout.readline()
                if raw_line == "":
                    break
                self._handle_raw_line(raw_line, source_process=process)
        except Exception as exc:  # noqa: BLE001 - reader thread reports failures through _closed_error.
            with self._condition:
                if self.process is process:
                    self._closed_error = f"reader failed: {exc}"
                    self._condition.notify_all()
        finally:
            with self._condition:
                if self.process is process:
                    self._initialized = False
                    self._accepting_since = None
                    if self._closed_error is None:
                        self._closed_error = "app-server exited"
                self._condition.notify_all()

    def _handle_raw_line(
        self,
        raw_line: str,
        *,
        source_process: ResidentProcess | None = None,
    ) -> None:
        classified = classify_app_server_transport_line(raw_line, decode_json_value=_decode_json_value)
        if classified.kind == "empty":
            return
        if classified.kind == "invalid-json":
            self._log(f"app_server_transport_non_json line={classified.invalid_preview!r}")
            return
        message = classified.message
        if message is None:
            return
        message_id = classified.message_id
        with self._condition:
            if source_process is not None and self.process is not source_process:
                self._log(
                    "app_server_transport_stale_reader_line_discarded "
                    + f"source_pid={source_process.pid}"
                )
                return
            if classified.kind == "server-request" and message_id is not None:
                self._pending.record_server_request(message_id, message, self._log)
                self._condition.notify_all()
            elif classified.kind == "response" and message_id is not None:
                if message_id != self._active_request_id:
                    self._resolve_ambiguous_turn_start_from_late_response(
                        message_id,
                        message,
                    )
                    self._reconcile_timed_out_thread_read_from_late_response(
                        message_id,
                        message,
                    )
                    self._log(f"app_server_transport_late_response_discarded id={message_id}")
                else:
                    self._responses[message_id] = message
                self._condition.notify_all()
            else:
                self._children.record_notification(
                    message,
                    generation=self._generation,
                    log=self._log,
                )
                self._pending.record_notification(message, self._log, now=self.monotonic_func())
                self._resolve_ambiguous_turn_start_from_notification(message)
                self._condition.notify_all()
        self._cleanup_retry.wake()
        self._restart_retry.wake()

    def _resolve_ambiguous_turn_start_from_notification(
        self,
        message: JsonObject,
    ) -> None:
        ambiguous_thread_id = self._ambiguous_turn_start_thread_id
        if ambiguous_thread_id is None:
            return
        method = str(message.get("method") or "")
        if method not in ("turn/started", "turn/completed"):
            return
        params = message.get("params")
        if not isinstance(params, dict):
            return
        thread_id = str(params.get("threadId") or "")
        if thread_id == ambiguous_thread_id:
            self._clear_ambiguous_turn_start()
            self._log(
                "app_server_ambiguous_turn_start_resolved "
                + f"thread={thread_id} notification={method}"
            )

    def _resolve_ambiguous_turn_start_from_late_response(
        self,
        request_id: str,
        message: JsonObject,
    ) -> None:
        if request_id != self._ambiguous_turn_start_request_id:
            return
        thread_id = self._ambiguous_turn_start_thread_id
        result = message.get("result")
        if thread_id is not None and isinstance(result, dict):
            turn = result.get("turn")
            if isinstance(turn, dict):
                turn_id = str(turn.get("id") or "").strip()
                if turn_id:
                    self._pending.record_notification(
                        {
                            "method": "turn/started",
                            "params": {
                                "threadId": thread_id,
                                "turnId": turn_id,
                            },
                        },
                        self._log,
                        now=self.monotonic_func(),
                    )
        self._clear_ambiguous_turn_start()
        outcome = "error" if "error" in message else "result"
        self._log(
            "app_server_ambiguous_turn_start_late_response "
            + f"thread={thread_id or '-'} outcome={outcome}"
        )

    def _clear_ambiguous_turn_start(self) -> None:
        self._ambiguous_turn_start_thread_id = None
        self._ambiguous_turn_start_request_id = None
        self._ambiguous_turn_start_deadline = None

    def _reconcile_timed_out_thread_read_from_late_response(
        self,
        request_id: str,
        message: JsonObject,
    ) -> None:
        timed_out_read = self._timed_out_thread_reads.pop(request_id, None)
        if timed_out_read is None:
            return
        thread_id, turn_id = timed_out_read
        self._record_thread_read_response()
        result = message.get("result")
        if not isinstance(result, dict):
            return
        thread = result.get("thread")
        if not isinstance(thread, dict) or str(thread.get("id") or "").strip() != thread_id:
            return
        if get_thread_status_type(thread) not in {"idle", "notLoaded"}:
            return
        _ = self._pending.reconcile_inactive_thread(thread_id, turn_id, self._log)

    def _record_thread_read_response(self) -> None:
        self._consecutive_read_timeouts = 0

    def _clear_timed_out_thread_reads(self) -> None:
        self._timed_out_thread_reads.clear()
        self._record_thread_read_response()

    def close_locked(self) -> None:
        process = self.process
        self._initialized = False
        self._accepting_since = None
        if process is None:
            self._children.reset(self._generation)
            self._subscriptions.clear()
            self._cleanup_retry.stop()
            return
        close_resident_app_server_process(process, self._log)
        stderr_recorder = self._stderr_recorder
        if stderr_recorder is not None:
            stderr_recorder.close()
        self.process = None
        self._stdout_thread = None
        self._stderr_recorder = None
        self._children.reset(self._generation)
        self._subscriptions.clear()
        self._cleanup_retry.stop()

    def close(self) -> None:
        with self._recycle_lock:
            with self._condition:
                self._closing = True
            self._restart_retry.stop()
            with self._request_lock:
                with self._condition:
                    self.close_locked()
                    self._draining = False
                    self._closed_error = "app-server closed"
                    self._condition.notify_all()

    def restart(self) -> None:
        try:
            with self._request_lock:
                with self._condition:
                    self._draining = True
                    self.close_locked()
                    self._closed_error = "app-server restarting"
                    self._condition.notify_all()
                self._start_with_request_slot_acquired()
        except Exception:
            with self._condition:
                self._draining = False
                self._condition.notify_all()
            raise

    @contextmanager
    def delivery_admission(
        self,
        expected_generation: int | None = None,
    ) -> Generator[AppServerLifecycleSnapshot, None, None]:
        leased = False
        with self._condition:
            snapshot = self.lifecycle_snapshot()
            if snapshot.quarantined or snapshot.restart_pending:
                if expected_generation is not None:
                    raise AppServerGenerationMismatch(
                        expected_generation=expected_generation,
                        actual_generation=snapshot.generation,
                        healthy=False,
                    )
            elif expected_generation is not None and (
                not snapshot.healthy or snapshot.generation != expected_generation
            ):
                raise AppServerGenerationMismatch(
                    expected_generation=expected_generation,
                    actual_generation=snapshot.generation,
                    healthy=snapshot.healthy,
                )
            elif snapshot.healthy:
                self._active_deliveries += 1
                leased = True
                self._delivery_local.lease_count = (
                    getattr(self._delivery_local, "lease_count", 0) + 1
                )
        try:
            yield snapshot
        finally:
            if leased:
                self._delivery_local.lease_count -= 1
                with self._condition:
                    self._active_deliveries -= 1
                    self._condition.notify_all()
                self.notify_child_cleanup_blocker_changed()

    def set_external_work_guard(self, guard: ExternalWorkGuard | None) -> None:
        with self._condition:
            self._external_work_guard = guard
        self.notify_child_cleanup_blocker_changed()

    def notify_child_cleanup_blocker_changed(self) -> None:
        snapshot = self.child_lifecycle_snapshot()
        if snapshot.cleanup_pending:
            self._cleanup_retry.schedule(snapshot.generation)
        else:
            self._cleanup_retry.wake()
        lifecycle = self.lifecycle_snapshot()
        if lifecycle.restart_pending:
            self._restart_retry.schedule(lifecycle.generation)
        else:
            self._restart_retry.wake()

    def child_lifecycle_snapshot(self) -> ChildLifecycleSnapshot:
        with self._lock:
            return self._children.snapshot(self._generation)

    def try_recycle_child_cleanup(
        self,
        *,
        expected_generation: int | None = None,
    ) -> ChildCleanupRecycleOutcome:
        try:
            outcome = self._try_recycle_child_cleanup(
                expected_generation=expected_generation
            )
        except (CodexAppServerTransportError, OSError, TimeoutError):
            snapshot = self.child_lifecycle_snapshot()
            if snapshot.cleanup_pending:
                self._cleanup_retry.schedule(snapshot.generation)
            raise
        if outcome.status not in (
            ChildCleanupRecycleStatus.RECYCLED,
            ChildCleanupRecycleStatus.NO_CLEANUP_DEBT,
        ):
            self._cleanup_retry.schedule(outcome.generation)
        return outcome

    def _retry_child_cleanup_once(self, generation: int) -> ChildCleanupRecycleOutcome:
        return self._try_recycle_child_cleanup(expected_generation=generation)

    def _try_recycle_child_cleanup(
        self,
        *,
        expected_generation: int | None,
    ) -> ChildCleanupRecycleOutcome:
        if not self._recycle_lock.acquire(blocking=False):
            return self._recycle_outcome(ChildCleanupRecycleStatus.RECYCLE_BUSY)
        try:
            snapshot = self.lifecycle_snapshot()
            if (
                expected_generation is not None
                and snapshot.generation != expected_generation
            ):
                raise AppServerGenerationMismatch(
                    expected_generation=expected_generation,
                    actual_generation=snapshot.generation,
                    healthy=snapshot.healthy,
                )
            if not self._children.snapshot(snapshot.generation).cleanup_pending:
                return ChildCleanupRecycleOutcome(
                    ChildCleanupRecycleStatus.NO_CLEANUP_DEBT,
                    snapshot.generation,
                )
            guard = self._external_work_guard
            if guard is not None:
                try:
                    if guard():
                        return self._recycle_outcome(ChildCleanupRecycleStatus.EXTERNAL_WORK)
                except Exception as exc:  # noqa: BLE001 - guard failure must defer cleanup, never permit it.
                    self._log(
                        "app_server_child_cleanup_guard_failed "
                        + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
                    )
                    return self._recycle_outcome(ChildCleanupRecycleStatus.EXTERNAL_WORK)
            if not self._request_lock.acquire(blocking=False):
                return self._recycle_outcome(ChildCleanupRecycleStatus.RECYCLE_BUSY)
            try:
                with self._condition:
                    if (
                        expected_generation is not None
                        and self._generation != expected_generation
                    ):
                        raise AppServerGenerationMismatch(
                            expected_generation=expected_generation,
                            actual_generation=self._generation,
                            healthy=self._initialized and self._is_running(),
                        )
                    if not self._children.snapshot(self._generation).cleanup_pending:
                        return self._recycle_outcome(
                            ChildCleanupRecycleStatus.NO_CLEANUP_DEBT
                        )
                    blocker = self._child_recycle_blocker_locked()
                    if blocker is not None:
                        return self._recycle_outcome(blocker)
                    old_generation = self._generation
                    self._draining = True
                    self._condition.notify_all()
            finally:
                self._request_lock.release()
            self._log(f"app_server_child_cleanup_recycle_started generation={old_generation}")
            self.restart()
            new_generation = self.lifecycle_snapshot().generation
            self._log(
                "app_server_child_cleanup_recycled "
                + f"old_generation={old_generation} new_generation={new_generation}"
            )
            return ChildCleanupRecycleOutcome(
                ChildCleanupRecycleStatus.RECYCLED,
                new_generation,
            )
        finally:
            self._recycle_lock.release()

    def try_restart_if_quiescent(self) -> bool:
        snapshot = self.lifecycle_snapshot()
        if snapshot.restart_pending:
            return self._try_restart_pending_generation(snapshot.generation)
        if not self._recycle_lock.acquire(blocking=False):
            self._log("app_server_refresh_deferred reason=recycle_busy")
            return False
        try:
            if self._external_work_blocks_restart():
                self._log("app_server_refresh_deferred reason=external_work")
                return False
            if not self._request_lock.acquire(blocking=False):
                self._log("app_server_refresh_deferred reason=request_busy")
                return False
            try:
                with self._condition:
                    blocker = self._child_recycle_blocker_locked()
                    if blocker is not None:
                        self._log(f"app_server_refresh_deferred reason={blocker.value}")
                        return False
                    old_generation = self._generation
                    self._draining = True
                    self._condition.notify_all()
            finally:
                self._request_lock.release()
            self._log(f"app_server_refresh_started generation={old_generation}")
            self.restart()
            self._log(
                "app_server_refresh_completed "
                + f"old_generation={old_generation} new_generation={self._generation}"
            )
            return True
        finally:
            self._recycle_lock.release()

    def _external_work_blocks_restart(self) -> bool:
        guard = self._external_work_guard
        if guard is None:
            return False
        try:
            return guard()
        except Exception as exc:  # noqa: BLE001 - a failed safety guard must prevent restart.
            self._log(
                "app_server_child_cleanup_guard_failed "
                + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
            )
            return True

    def _retry_restart_pending_once(self, generation: int) -> bool:
        return self._try_restart_pending_generation(generation)

    def _try_restart_pending_generation(
        self,
        generation: int,
        *,
        active_delivery_allowance: int = 0,
    ) -> bool:
        if not self._recycle_lock.acquire(blocking=False):
            return False
        try:
            snapshot = self.lifecycle_snapshot()
            if snapshot.generation != generation:
                return True
            if not snapshot.restart_pending:
                return True
            if self._closing:
                return True
            if self._external_work_blocks_restart():
                return False
            if not self._request_lock.acquire(blocking=False):
                return False
            try:
                with self._condition:
                    if self._generation != generation:
                        return True
                    blocker = self._pending_restart_blocker_locked(
                        active_delivery_allowance=active_delivery_allowance
                    )
                    if blocker is not None:
                        self._log(
                            "app_server_restart_pending_deferred "
                            + f"generation={generation} reason={blocker.value}"
                        )
                        return False
                    self._draining = True
                    self.close_locked()
                    self._closed_error = "app-server restarting"
                    self._condition.notify_all()
                self._log(f"app_server_restart_pending_started generation={generation}")
                self._start_with_request_slot_acquired()
            finally:
                self._request_lock.release()
            self._log(
                "app_server_restart_pending_completed "
                + f"old_generation={generation} new_generation={self._generation}"
            )
            return True
        finally:
            self._recycle_lock.release()

    def _pending_restart_blocker_locked(
        self,
        *,
        active_delivery_allowance: int = 0,
    ) -> ChildCleanupRecycleStatus | None:
        if self._active_deliveries > active_delivery_allowance:
            return ChildCleanupRecycleStatus.ACTIVE_DELIVERY
        if self._pending.has_active_turns():
            return ChildCleanupRecycleStatus.ACTIVE_TURN
        if self._ambiguous_turn_start_thread_id is not None:
            deadline = self._ambiguous_turn_start_deadline
            if deadline is None or self.monotonic_func() < deadline:
                return ChildCleanupRecycleStatus.PENDING_REQUEST
            thread_id = self._ambiguous_turn_start_thread_id
            self._clear_ambiguous_turn_start()
            self._log(
                "app_server_ambiguous_turn_start_grace_expired "
                + f"thread={thread_id}"
            )
        if self._pending.has_pending_server_requests() or self._active_request_id is not None:
            return ChildCleanupRecycleStatus.PENDING_REQUEST
        return None

    def _child_recycle_blocker_locked(self) -> ChildCleanupRecycleStatus | None:
        if self._active_deliveries:
            return ChildCleanupRecycleStatus.ACTIVE_DELIVERY
        if self._pending.has_active_turns():
            return ChildCleanupRecycleStatus.ACTIVE_TURN
        if self._pending.has_pending_server_requests() or self._active_request_id is not None:
            return ChildCleanupRecycleStatus.PENDING_REQUEST
        if self._subscriptions.has_subscriptions():
            return ChildCleanupRecycleStatus.SUBSCRIBED_THREAD
        return None

    def _recycle_outcome(self, status: ChildCleanupRecycleStatus) -> ChildCleanupRecycleOutcome:
        generation = self.lifecycle_snapshot().generation
        self._log(f"app_server_child_cleanup_deferred generation={generation} reason={status.value}")
        return ChildCleanupRecycleOutcome(status, generation)

    def thread_subscription_lock(self, thread_id: str) -> AbstractContextManager[None]:
        return self._subscriptions.lifecycle_lock(thread_id)

    def is_thread_subscribed(self, thread_id: str) -> bool:
        return self._subscriptions.is_subscribed(thread_id)

    def mark_thread_subscribed(self, thread_id: str) -> None:
        self._subscriptions.mark_subscribed(thread_id)

    def note_thread_activity(self, thread_id: str) -> None:
        self._subscriptions.note_thread_activity(thread_id)

    def request(
        self,
        method: str,
        params: JsonMapping | None = None,
        *,
        timeout_sec: float = 10.0,
        expected_generation: int | None = None,
        recovery_timeout_sec: float | None = None,
    ) -> JsonObject:
        initial_snapshot = self.lifecycle_snapshot()
        request_generation = initial_snapshot.generation
        if expected_generation is None and not initial_snapshot.restart_pending:
            self.start()
        request_params = params or {}
        fresh_read_timeout = min(
            _MAX_FRESH_READ_RETRY_SECONDS,
            max(
                0.0,
                _MAX_FRESH_READ_RETRY_SECONDS
                if recovery_timeout_sec is None
                else recovery_timeout_sec,
            ),
        )
        pending_snapshot = self.lifecycle_snapshot()
        if pending_snapshot.restart_pending:
            if expected_generation is not None or fresh_read_timeout <= 0:
                raise AppServerGenerationQuarantinedError(
                    generation=pending_snapshot.generation
                )
            fresh_generation = self._prepare_fresh_read_generation(
                method,
                pending_snapshot.generation,
            )
            if fresh_generation is None:
                raise AppServerGenerationQuarantinedError(
                    generation=pending_snapshot.generation
                )
            return self._request_started(
                method,
                request_params,
                timeout_sec=fresh_read_timeout,
                expected_generation=fresh_generation,
            )
        try:
            return self._request_started(
                method,
                request_params,
                timeout_sec=timeout_sec,
                expected_generation=expected_generation,
            )
        except TimeoutError:
            if method == "thread/read":
                raise
            if expected_generation is not None or fresh_read_timeout <= 0:
                raise
            fresh_generation = self._prepare_fresh_read_generation(
                method,
                request_generation,
            )
            if fresh_generation is None:
                raise
            return self._request_started(
                method,
                request_params,
                timeout_sec=fresh_read_timeout,
                expected_generation=fresh_generation,
            )

    def _prepare_fresh_read_generation(
        self,
        method: str,
        timed_out_generation: int,
    ) -> int | None:
        if method != "thread/read":
            return None
        snapshot = self.lifecycle_snapshot()
        if snapshot.generation == timed_out_generation and not self._try_restart_pending_generation(
            timed_out_generation,
            active_delivery_allowance=(
                1 if getattr(self._delivery_local, "lease_count", 0) > 0 else 0
            ),
        ):
            return None
        fresh = self.lifecycle_snapshot()
        if not fresh.healthy or fresh.generation == timed_out_generation:
            return None
        self._log(
            "app_server_thread_read_retry_fresh_generation "
            + f"old_generation={timed_out_generation} new_generation={fresh.generation}"
        )
        return fresh.generation

    def _quarantine_after_response_timeout(
        self,
        method: str,
        params: JsonMapping,
        request_id: str,
    ) -> None:
        with self._condition:
            generation = self._generation
            if method == "thread/read":
                self._consecutive_read_timeouts += 1
                thread_id = str(params.get("threadId") or "").strip()
                turn_id = self._pending.active_turn_id(thread_id)
                if thread_id and turn_id:
                    if len(self._timed_out_thread_reads) >= _MAX_TIMED_OUT_THREAD_READS:
                        oldest_request_id = next(iter(self._timed_out_thread_reads))
                        del self._timed_out_thread_reads[oldest_request_id]
                    self._timed_out_thread_reads[request_id] = (thread_id, turn_id)
                self._condition.notify_all()
                self._log(
                    "app_server_thread_read_timeout_isolated "
                    + f"generation={generation} consecutive={self._consecutive_read_timeouts}"
                )
                return
            self._quarantined_generation = generation
            self._restart_pending = True
            if method == "turn/start":
                thread_id = str(params.get("threadId") or "").strip()
                self._ambiguous_turn_start_thread_id = thread_id or "unknown"
                self._ambiguous_turn_start_request_id = request_id
                self._ambiguous_turn_start_deadline = (
                    self.monotonic_func() + _AMBIGUOUS_TURN_START_GRACE_SECONDS
                )
            self._condition.notify_all()
        self._log(
            "app_server_response_timeout_quarantined "
            + f"method={method} generation={generation}"
        )
        self._restart_retry.schedule(generation)

    def _request_started(
        self,
        method: str,
        params: JsonMapping,
        *,
        timeout_sec: float,
        expected_generation: int | None = None,
        _request_slot_acquired: bool = False,
    ) -> JsonObject:
        deadline = self.monotonic_func() + max(timeout_sec, 0.0)
        if not _request_slot_acquired:
            lock_timeout = max(0.0, deadline - self.monotonic_func())
            if not self._request_lock.acquire(timeout=lock_timeout):
                raise TimeoutError(f"Timed out waiting for resident app-server request slot for {method}.")
        request_id: str | None = None
        try:
            if expected_generation is not None:
                self._raise_for_generation_mismatch(expected_generation)
            if not self._is_running():
                raise CodexAppServerTransportError("Resident Codex app-server is not running.")
            if self.monotonic_func() >= deadline:
                raise TimeoutError(f"Timed out waiting for resident app-server request slot for {method}.")
            request_id = str(uuid.uuid4())
            payload: JsonObject = {
                "id": request_id,
                "method": method,
                "params": dict(params),
            }
            with self._condition:
                self._active_request_id = request_id
            self._write_message(payload)
            with self._condition:
                while True:
                    response = self._responses.pop(request_id, None)
                    if response is not None:
                        if method == "thread/read":
                            self._record_thread_read_response()
                        return extract_response_result(method, response)
                    if self._closed_error and not self._is_running():
                        raise CodexAppServerTransportError(
                            f"Codex app-server exited while waiting for {method}: {self._closed_error}"
                        )
                    remaining = deadline - self.monotonic_func()
                    if remaining <= 0:
                        self._quarantine_after_response_timeout(
                            method,
                            params,
                            request_id,
                        )
                        raise TimeoutError(f"Timed out waiting for app-server response to {method}.")
                    _ = self._condition.wait(timeout=min(remaining, 0.5))
        finally:
            if request_id is not None:
                with self._condition:
                    if self._active_request_id == request_id:
                        self._active_request_id = None
                    _ = self._responses.pop(request_id, None)
            if not _request_slot_acquired:
                self._request_lock.release()
            self._cleanup_retry.wake()
            self._restart_retry.wake()

    def notify(self, method: str, params: JsonMapping | None = None) -> None:
        self._write_message({"method": method, "params": dict(params or {})})

    def _write_message(self, payload: JsonMapping) -> None:
        process = self.process
        stdin = process.stdin if process is not None else None
        if stdin is None or stdin.closed:
            raise CodexAppServerTransportError("Resident Codex app-server stdin is closed.")
        with self._write_lock:
            _ = stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
            stdin.flush()

    def respond_to_server_request(self, request_id: str, result: JsonMapping) -> None:
        if not self._is_running():
            raise CodexAppServerTransportError("Cannot answer app-server request because the server is not running.")
        self._write_message({"id": request_id, "result": dict(result)})
        with self._condition:
            self._pending.resolve_request(request_id)
        self._log(f"app_server_request_resolved id={request_id}")
        self.notify_child_cleanup_blocker_changed()

    def get_pending_server_requests(self, thread_id: str | None = None) -> list[JsonObject]:
        with self._lock:
            return self._pending.pending_requests(thread_id)

    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None:
        with self._lock:
            return self._pending.latest_approval_request(thread_id)

    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None:
        with self._lock:
            return self._pending.latest_input_request(thread_id)

    def observe_turn_completion(self, thread_id: str, turn_id: str) -> TurnCompletionObservation:
        with self._lock:
            completion = self._pending.turn_completion(thread_id, turn_id)
            if completion is not None:
                return TurnCompletionFound(completion)
            if self._closed_error is not None and not self._is_running():
                return TurnCompletionTransportError(self._closed_error)
            return TurnCompletionPending()

    def wait_for_turn_completion(
        self,
        thread_id: str,
        turn_id: str,
        *,
        timeout_sec: float,
        expected_generation: int | None = None,
    ) -> TurnCompletionObservation:
        deadline = self.monotonic_func() + max(0.0, timeout_sec)
        with self._condition:
            while True:
                if expected_generation is not None:
                    self._raise_for_generation_mismatch(expected_generation)
                observation = self.observe_turn_completion(thread_id, turn_id)
                if not isinstance(observation, TurnCompletionPending):
                    return observation
                remaining = deadline - self.monotonic_func()
                if remaining <= 0:
                    return observation
                _ = self._condition.wait(timeout=min(remaining, 0.5))

    def register_remote_interrupt_intent(self, thread_id: str, turn_id: str) -> bool:
        with self._condition:
            registered = self._pending.register_remote_interrupt_intent(
                thread_id,
                turn_id,
                registered_at=self.monotonic_func(),
            )
            self._condition.notify_all()
            return registered

    def cancel_remote_interrupt_intent(self, thread_id: str, turn_id: str) -> None:
        with self._condition:
            self._pending.cancel_remote_interrupt_intent(thread_id, turn_id)
            self._condition.notify_all()

    def get_cached_goal_update(self, thread_id: str, turn_id: str) -> ThreadGoalUpdate | None:
        with self._lock:
            return self._pending.goal_update(thread_id, turn_id)

    def get_cached_turn_completion(self, thread_id: str, turn_id: str) -> TurnCompletion | None:
        with self._lock:
            return self._pending.turn_completion(thread_id, turn_id)

    def get_cached_thread_turn_completions(self, thread_id: str) -> dict[str, TurnCompletion]:
        with self._lock:
            return self._pending.turn_completions_for_thread(thread_id)

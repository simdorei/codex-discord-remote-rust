from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import os
from collections.abc import Callable, Mapping
from contextlib import AbstractContextManager
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType

import codex_app_server_transport as app_server_transport
import codex_discord_runtime as discord_runtime
import codex_discord_runtime_lock as discord_runtime_lock
import codex_discord_runner_queue as discord_runner_queue
import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_output_targets as discord_session_mirror_output_targets
from codex_remote_mcp_binding import close_remote_mcp_bridge

LogFunc = Callable[[str], None]
GetPathFunc = Callable[[], Path]
ExitProcessFunc = Callable[[int], None]
CloseAppServerFunc = Callable[[], None]
CloseRemoteBridgeFunc = Callable[[], None]
ExpiredOutputTargetHandler = Callable[[str, float], None]
RunnerRecord = discord_runtime.RunnerState | discord_runner_queue.ThreadRunner


class RunnerSnapshotLock:
    async def __aenter__(self) -> None:
        return None

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None:
        _ = (exc_type, exc_value, traceback)
        return None


@dataclass(frozen=True, slots=True)
class RuntimeStateBridge:
    session_mirror_state: discord_session_mirror.SessionMirrorState
    runtime_state: discord_runtime.DiscordRuntimeState
    thread_runners: Mapping[str, RunnerRecord]
    thread_runners_lock: asyncio.Lock
    active_output_ttl_seconds: float
    runtime_mutex_name: str
    get_runtime_lock_path: GetPathFunc
    log: LogFunc
    on_expired_output_target: ExpiredOutputTargetHandler | None = None
    close_remote_bridge: CloseRemoteBridgeFunc = close_remote_mcp_bridge
    close_app_server: CloseAppServerFunc = app_server_transport.DEFAULT_CLIENT.close
    exit_process: ExitProcessFunc = os._exit

    def get_session_mirror_state(self) -> discord_session_mirror.SessionMirrorState:
        return self.session_mirror_state

    def get_runtime_state(self) -> discord_runtime.DiscordRuntimeState:
        return self.runtime_state

    async def is_thread_runner_busy(self, target_thread_id: str | None) -> bool:
        key = discord_runtime.normalize_runner_key(target_thread_id)
        if discord_session_mirror_output_targets.is_active_session_mirror_output_target(
            self.session_mirror_state,
            target_thread_id,
            active_ttl_seconds=self.active_output_ttl_seconds,
            on_expired=self.on_expired_output_target,
        ):
            return True
        async with self.runtime_state.active_direct_ask_lock:
            if key in self.runtime_state.active_direct_ask_target_keys:
                return True
        delivery_lock = self.runtime_state.ask_delivery_locks.get(key)
        if delivery_lock is not None and delivery_lock.locked():
            return True
        async with self.thread_runners_lock:
            runner = self.thread_runners.get(key)
            if runner is None:
                return False
            queue = runner.get("queue")
            return bool(runner.get("active")) or (
                isinstance(queue, asyncio.Queue) and queue.qsize() > 0
            )

    async def claim_direct_ask_target(self, target_thread_id: str | None) -> bool:
        key = discord_runtime.normalize_runner_key(target_thread_id)
        async with self.runtime_state.active_direct_ask_lock:
            if key in self.runtime_state.active_direct_ask_target_keys:
                return False
            self.runtime_state.active_direct_ask_target_keys.add(key)
            return True

    async def release_direct_ask_target(self, target_thread_id: str | None) -> None:
        key = discord_runtime.normalize_runner_key(target_thread_id)
        async with self.runtime_state.active_direct_ask_lock:
            self.runtime_state.active_direct_ask_target_keys.discard(key)

    def get_ask_delivery_lock(self, target_thread_id: str | None) -> asyncio.Lock:
        key = discord_runtime.normalize_runner_key(target_thread_id)
        lock = self.runtime_state.ask_delivery_locks.get(key)
        if lock is None:
            lock = asyncio.Lock()
            self.runtime_state.ask_delivery_locks[key] = lock
        return lock

    def mark_steering_handoff(self, target_thread_id: str | None) -> float:
        return discord_runtime.mark_steering_handoff(
            self.runtime_state.steering_handoffs,
            target_thread_id,
        )

    def had_steering_handoff_since(self, target_thread_id: str | None, started_at: float) -> bool:
        return discord_runtime.had_steering_handoff_since(
            self.runtime_state.steering_handoffs,
            target_thread_id,
            started_at,
        )

    def register_discord_relay(self, target_thread_id: str | None) -> int:
        return discord_runtime.register_discord_relay(
            self.runtime_state.active_discord_relay_generations,
            target_thread_id,
        )

    def is_discord_relay_stale(self, target_thread_id: str | None, generation: int) -> bool:
        return discord_runtime.is_discord_relay_stale(
            self.runtime_state.active_discord_relay_generations,
            target_thread_id,
            generation,
        )

    def snapshot_thread_runners(self) -> dict[str, discord_runtime.RunnerState]:
        snapshots: dict[str, discord_runtime.RunnerState] = {}
        for key, runner in self.thread_runners.items():
            snapshot: discord_runtime.RunnerState = {}
            if "queue" in runner:
                snapshot["queue"] = runner["queue"]
            if "active" in runner:
                snapshot["active"] = runner["active"]
            if "target_thread_id" in runner:
                snapshot["target_thread_id"] = runner["target_thread_id"]
            snapshots[key] = snapshot
        return snapshots

    def acquire_runtime_instance_lock(self, mutex_name: str | None = None) -> AbstractContextManager[bool]:
        active_mutex_name = self.runtime_mutex_name if mutex_name is None else mutex_name
        return discord_runtime_lock.acquire_runtime_instance_lock(
            active_mutex_name,
            runtime_mutex_name=self.runtime_mutex_name,
            runtime_lock_path=self.get_runtime_lock_path(),
            log_func=self.log,
            remove_runtime_lock_func=self.remove_runtime_lock_for_current_process,
        )

    def remove_runtime_lock_for_current_process(self, *, reason: str) -> None:
        discord_runtime_lock.remove_runtime_lock_for_current_process(
            self.get_runtime_lock_path(),
            reason=reason,
            log_func=self.log,
        )

    def exit_bot_process(self, exit_code: int, *, reason: str) -> None:
        self.log(f"bot_process_exit_requested reason={reason} code={exit_code}")
        for service_name, close_service in (
            ("remote_bridge", self.close_remote_bridge),
            ("app_server", self.close_app_server),
        ):
            try:
                close_service()
            except BaseException as exc:  # noqa: BLE001 - forced exit must attempt every cleanup.
                self.log(
                    f"bot_process_{service_name}_close_failed "
                    + f"reason={reason} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
        self.remove_runtime_lock_for_current_process(reason=reason)
        self.exit_process(exit_code)

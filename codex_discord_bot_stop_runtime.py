from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Coroutine
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias

import codex_discord_stop_marker as discord_stop_marker
from codex_process_identity import current_process_identity

ModuleValue: TypeAlias = object


class StopRuntimeOwner(Protocol):
    def is_closed(self) -> bool: ...

    async def stop_marker_loop(self) -> None: ...

    async def close(self) -> None: ...


@dataclass(frozen=True, slots=True)
class BotStopRuntimeDeps:
    get_stop_request_path: Callable[[], Path]
    get_poll_seconds: Callable[[], float]
    get_drain_timeout_seconds: Callable[[], float]
    get_close_timeout_seconds: Callable[[], float]
    create_task: Callable[
        [Coroutine[ModuleValue, ModuleValue, None]], asyncio.Task[None]
    ]
    wait_for: Callable[[Awaitable[None], float], Awaitable[None]]
    sleep: Callable[[float], Awaitable[None]]
    set_delivery_stopping: discord_stop_marker.SetDeliveryStopping
    wait_for_delivery_drain: discord_stop_marker.WaitForDeliveryDrain
    exit_bot_process: discord_stop_marker.ExitBotProcess
    log: Callable[[str], None]
    prepare_restart_handoff: Callable[[], bool] = lambda: False


@dataclass(frozen=True, slots=True)
class BotStopRuntime:
    deps: BotStopRuntimeDeps

    async def start_stop_marker_watcher(self, owner: StopRuntimeOwner) -> None:
        task = _stop_marker_task(owner)
        if task and not task.done():
            self.deps.log("stop_marker_watcher_already_running")
            return
        setattr(
            owner, "_stop_marker_task", self.deps.create_task(owner.stop_marker_loop())
        )
        self.deps.log(
            f"stop_marker_watcher_started seconds={self.deps.get_poll_seconds():g}"
        )

    async def stop_marker_loop(self, owner: StopRuntimeOwner) -> None:
        close_timeout_seconds = self.deps.get_close_timeout_seconds()
        stop_request_path = self.deps.get_stop_request_path()
        await discord_stop_marker.stop_marker_loop(
            discord_stop_marker.StopMarkerLoopDeps(
                stop_request_path=stop_request_path,
                heartbeat_path=stop_request_path.with_name(
                    ".codex_discord_bot.heartbeat"
                ),
                process_identity=current_process_identity(),
                poll_seconds=self.deps.get_poll_seconds(),
                drain_timeout_seconds=self.deps.get_drain_timeout_seconds(),
                close_timeout_seconds=close_timeout_seconds,
                is_closed=owner.is_closed,
                set_delivery_stopping=self.deps.set_delivery_stopping,
                wait_for_delivery_drain=self.deps.wait_for_delivery_drain,
                close_with_timeout=lambda: self.deps.wait_for(
                    owner.close(), close_timeout_seconds
                ),
                sleep=self.deps.sleep,
                exit_bot_process=self.deps.exit_bot_process,
                log=self.deps.log,
                prepare_restart_handoff=self.deps.prepare_restart_handoff,
            )
        )


def _stop_marker_task(owner: StopRuntimeOwner) -> asyncio.Task[ModuleValue] | None:
    value = getattr(owner, "_stop_marker_task", None)
    if isinstance(value, asyncio.Task):
        return value
    return None

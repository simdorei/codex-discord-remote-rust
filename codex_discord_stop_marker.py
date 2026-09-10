from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, Protocol


LogFunc = Callable[[str], None]
SetDeliveryStopping = Callable[[str], None]
CloseWithTimeout = Callable[[], Awaitable[None]]
SleepFunc = Callable[[float], Awaitable[None]]
StopMode = Literal["stop", "restart"]


class WaitForDeliveryDrain(Protocol):
    def __call__(self, *, timeout_seconds: float, reason: str) -> Awaitable[bool]: ...


class ExitBotProcess(Protocol):
    def __call__(self, exit_code: int, *, reason: str) -> None: ...


@dataclass(frozen=True, slots=True)
class StopMarkerLoopDeps:
    stop_request_path: Path
    heartbeat_path: Path
    process_identity: str
    poll_seconds: float
    drain_timeout_seconds: float
    close_timeout_seconds: float
    is_closed: Callable[[], bool]
    set_delivery_stopping: SetDeliveryStopping
    wait_for_delivery_drain: WaitForDeliveryDrain
    close_with_timeout: CloseWithTimeout
    sleep: SleepFunc
    exit_bot_process: ExitBotProcess
    log: LogFunc
    prepare_restart_handoff: Callable[[], bool] = lambda: False


async def stop_marker_loop(deps: StopMarkerLoopDeps) -> None:
    while not deps.is_closed():
        try:
            deps.heartbeat_path.touch()
        except OSError as exc:
            deps.log(
                f"heartbeat_update_failed path={deps.heartbeat_path} "
                + f"error_type={type(exc).__name__}"
            )
        if deps.stop_request_path.exists():
            mode = _marker_mode(deps)
            if mode is None:
                await deps.sleep(deps.poll_seconds)
                continue
            prepare_restart = mode == "restart"
            deps.log(f"stop_marker_detected path={deps.stop_request_path}")
            try:
                deps.stop_request_path.unlink()
            except OSError as exc:
                deps.log(
                    f"stop_marker_remove_failed path={deps.stop_request_path} "
                    + f"error_type={type(exc).__name__}"
                )
            deps.set_delivery_stopping("stop_marker")
            _ = await deps.wait_for_delivery_drain(
                timeout_seconds=deps.drain_timeout_seconds,
                reason="stop_marker",
            )
            if prepare_restart:
                prepared = deps.prepare_restart_handoff()
                deps.log(f"remote_mcp_restart_handoff_ready prepared={prepared}")
            deps.log(
                f"stop_marker_close_start timeout_seconds={deps.close_timeout_seconds:g}"
            )
            try:
                await deps.close_with_timeout()
            except TimeoutError:
                deps.log(
                    f"stop_marker_close_timeout timeout_seconds={deps.close_timeout_seconds:g}"
                )
                deps.exit_bot_process(0, reason="stop_marker_close_timeout")
                return
            deps.log("stop_marker_close_done")
            return
        await deps.sleep(deps.poll_seconds)


def _marker_mode(deps: StopMarkerLoopDeps) -> StopMode | None:
    try:
        marker = deps.stop_request_path.read_text(encoding="utf-8").strip()
    except OSError as exc:
        deps.log(
            f"stop_marker_read_failed path={deps.stop_request_path} "
            + f"error_type={type(exc).__name__}"
        )
        return None
    identity = f"identity={deps.process_identity}"
    if marker == identity:
        return "stop"
    if marker == f"{identity}\nmode=restart":
        return "restart"
    deps.log(f"stop_marker_stale path={deps.stop_request_path}")
    try:
        deps.stop_request_path.unlink()
    except OSError as exc:
        deps.log(
            f"stop_marker_remove_failed path={deps.stop_request_path} "
            + f"error_type={type(exc).__name__}"
        )
    return None

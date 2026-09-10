# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

import base64
import sys
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import TypeGuard, final
from uuid import uuid4

from codex_remote_mcp_computer_contracts import (
    ComputerActionPermit,
    ComputerPlatform,
    ComputerWindowIdentity,
)
from codex_remote_mcp_computer_errors import ComputerControlError
from simdorei_mcp_common.operation_outputs import (
    ComputerActionOutput,
    ComputerScreenshotOutput,
    ComputerStopOutput,
    ComputerWindowsOutput,
)
from simdorei_mcp_common.operation_requests import (
    ComputerActivateRequest,
    ComputerClickRequest,
    ComputerCloseRequest,
    ComputerDragRequest,
    ComputerLaunchRequest,
    ComputerListWindowsRequest,
    ComputerOperation,
    ComputerPressKeysRequest,
    ComputerScreenshotRequest,
    ComputerScrollRequest,
    ComputerSetClipboardRequest,
    ComputerStopRequest,
    ComputerTypeTextRequest,
    ProjectOperation,
)
from simdorei_mcp_common.request_deadlines import RequestBudget

Clock = Callable[[], float]
TokenFactory = Callable[[], str]
_CAPTURE_LOCK = threading.Lock()


@dataclass(frozen=True, slots=True)
class Observation:
    identity: ComputerWindowIdentity
    expires_at: float


@dataclass(frozen=True, slots=True)
class ActionPermit:
    identity: ComputerWindowIdentity
    expires_at: float
    clock: Clock
    stopped: threading.Event

    def require_active(self) -> None:
        if self.stopped.is_set():
            raise ComputerControlError("Computer control was stopped or rebound.")
        if self.expires_at <= self.clock():
            raise ComputerControlError("Take a fresh screenshot before this action.")


@final
class ComputerController:  # MUTABLE_OK: owns short-lived screenshot observations.
    def __init__(
        self,
        platform: ComputerPlatform,
        *,
        clock: Clock = time.monotonic,
        token_factory: TokenFactory = lambda: uuid4().hex,
        observation_ttl_seconds: float = 30.0,
    ) -> None:
        self._platform = platform
        self._clock = clock
        self._token_factory = token_factory
        self._observation_ttl_seconds = observation_ttl_seconds
        self._lock = threading.Lock()
        self._operation_lock = threading.RLock()
        self._observations: dict[str, Observation] = {}
        self._stopped = threading.Event()
        self._platform_stopped = False

    def list_windows(self) -> ComputerWindowsOutput:
        self.require_running()
        output = ComputerWindowsOutput(windows=self._platform.list_windows())
        self.require_running()
        return output

    def screenshot(self, window_id: int) -> ComputerScreenshotOutput:
        self.require_running()
        with _CAPTURE_LOCK:
            capture = self._platform.screenshot(window_id)
            observation_id = self._token_factory()
            observation = Observation(
                identity=capture.identity,
                expires_at=self._clock() + self._observation_ttl_seconds,
            )
            encoded = base64.b64encode(capture.png).decode("ascii")
            with self._lock:
                self.require_running()
                self._observations.clear()
                self._observations[observation_id] = observation
            return ComputerScreenshotOutput(
                observation_id=observation_id,
                window=capture.window,
                data_base64=encoded,
            )

    def consume_observation(
        self,
        observation_id: str,
        window_id: int,
        points: tuple[tuple[int, int], ...] = (),
    ) -> ComputerActionPermit:
        with self._lock:
            observation = self._observations.pop(observation_id, None)
        if observation is None or observation.expires_at <= self._clock():
            raise ComputerControlError("Take a fresh screenshot before this action.")
        if observation.identity.window_id != window_id:
            raise ComputerControlError("The screenshot belongs to a different window.")
        for x, y in points:
            if x >= observation.identity.width or y >= observation.identity.height:
                raise ComputerControlError("The requested point is outside the screenshot.")
        permit = ActionPermit(
            identity=observation.identity,
            expires_at=observation.expires_at,
            clock=self._clock,
            stopped=self._stopped,
        )
        permit.require_active()
        return permit

    def stop(self, *, deadline_monotonic: float | None = None) -> None:
        self._stopped.set()
        if deadline_monotonic is None:
            acquired = self._operation_lock.acquire()
        else:
            acquired = self._operation_lock.acquire(
                timeout=max(0.0, deadline_monotonic - time.monotonic())
            )
        if not acquired:
            raise TimeoutError("Timed out waiting to stop computer control.")
        try:
            with self._lock:
                self._observations.clear()
                should_stop_platform = not self._platform_stopped
            if should_stop_platform:
                if deadline_monotonic is None:
                    self._platform.stop()
                else:
                    self._platform.stop(deadline_monotonic=deadline_monotonic)
                with self._lock:
                    self._platform_stopped = True
        finally:
            self._operation_lock.release()

    def begin_operation(self) -> None:
        _ = self._operation_lock.acquire()
        try:
            self.require_running()
        except ComputerControlError:
            self._operation_lock.release()
            raise

    def end_operation(self) -> None:
        try:
            self.require_running()
        finally:
            self._operation_lock.release()

    def require_running(self) -> None:
        if self._stopped.is_set():
            raise ComputerControlError("Computer control was stopped or rebound.")

    @property
    def platform(self) -> ComputerPlatform:
        return self._platform


def execute_computer_operation(
    request: ComputerOperation,
    *,
    controller: ComputerController | None = None,
    budget: RequestBudget | None = None,
) -> ComputerWindowsOutput | ComputerScreenshotOutput | ComputerActionOutput | ComputerStopOutput:
    active = controller or default_computer_controller()
    if isinstance(request, ComputerStopRequest):
        deadline = (
            time.monotonic() + budget.remaining()
            if budget is not None
            else None
        )
        active.stop(deadline_monotonic=deadline)
        return ComputerStopOutput(
            message="Computer control stopped until this project is bound again."
        )
    active.begin_operation()
    try:
        from codex_remote_mcp_computer_execute import execute_running_operation

        return execute_running_operation(request, active, budget=budget)
    finally:
        active.end_operation()


_default_controller: ComputerController | None = None
_default_lock = threading.Lock()


def default_computer_controller() -> ComputerController:
    global _default_controller
    with _default_lock:
        if _default_controller is None:
            _default_controller = new_computer_controller()
        return _default_controller


def new_computer_controller() -> ComputerController:
    if sys.platform != "win32":
        raise ComputerControlError(
            "Computer control is currently available only on Windows."
        )
    from codex_remote_mcp_windows_platform import WindowsComputerPlatform

    return ComputerController(WindowsComputerPlatform())


def new_device_computer_controller() -> ComputerController:
    if sys.platform != "win32":
        raise ComputerControlError(
            "Full computer control is currently available only on Windows."
        )
    from codex_remote_mcp_windows_device_platform import WindowsDeviceComputerPlatform

    return ComputerController(WindowsDeviceComputerPlatform())


COMPUTER_REQUEST_TYPES = (
    ComputerListWindowsRequest,
    ComputerActivateRequest,
    ComputerLaunchRequest,
    ComputerScreenshotRequest,
    ComputerClickRequest,
    ComputerDragRequest,
    ComputerScrollRequest,
    ComputerTypeTextRequest,
    ComputerPressKeysRequest,
    ComputerCloseRequest,
    ComputerSetClipboardRequest,
    ComputerStopRequest,
)


def is_computer_operation(value: ProjectOperation) -> TypeGuard[ComputerOperation]:
    return isinstance(value, COMPUTER_REQUEST_TYPES)

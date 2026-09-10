from __future__ import annotations

import logging
from collections.abc import Callable
from typing import Protocol, assert_never

from codex_remote_mcp_computer_contracts import ComputerActionPermit, ComputerPlatform
from codex_remote_mcp_computer_errors import ComputerControlError
from simdorei_mcp_common.operation_outputs import (
    ComputerActionName,
    ComputerActionOutput,
    ComputerScreenshotOutput,
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
)
from simdorei_mcp_common.request_deadlines import RequestBudget

LOGGER = logging.getLogger(__name__)


class ComputerControllerLike(Protocol):
    @property
    def platform(self) -> ComputerPlatform: ...
    def list_windows(self) -> ComputerWindowsOutput: ...
    def screenshot(self, window_id: int) -> ComputerScreenshotOutput: ...
    def consume_observation(
        self,
        observation_id: str,
        window_id: int,
        points: tuple[tuple[int, int], ...] = (),
    ) -> ComputerActionPermit: ...
    def require_running(self) -> None: ...


def execute_running_operation(
    request: ComputerOperation,
    active: ComputerControllerLike,
    *,
    budget: RequestBudget | None = None,
) -> ComputerWindowsOutput | ComputerScreenshotOutput | ComputerActionOutput:
    ensure_active: Callable[[], None] | None = (
        budget.ensure_active if budget is not None else None
    )
    if ensure_active is not None:
        ensure_active()
    match request:
        case ComputerListWindowsRequest():
            return active.list_windows()
        case ComputerActivateRequest(window_id=window_id):
            active.require_running()
            window = active.platform.activate(window_id)
            active.require_running()
            return _action("activate", window.window_id, "Window activated.")
        case ComputerLaunchRequest(app=app):
            active.require_running()
            if ensure_active is None:
                active.platform.launch(app)
            else:
                active.platform.launch(app, ensure_active=ensure_active)
            if ensure_active is not None:
                ensure_active()
            active.require_running()
            return _action("launch", None, f"{app} launch requested.")
        case ComputerScreenshotRequest(window_id=window_id):
            return active.screenshot(window_id)
        case ComputerClickRequest() as click:
            permit = active.consume_observation(
                click.observation_id, click.window_id, ((click.x, click.y),)
            )
            active.platform.click(permit, click.x, click.y, click.button, click.click_count)
            permit.require_active()
            return _action("click", click.window_id, "Click sent.")
        case ComputerDragRequest() as drag:
            points = ((drag.start_x, drag.start_y), (drag.end_x, drag.end_y))
            permit = active.consume_observation(drag.observation_id, drag.window_id, points)
            active.platform.drag(
                permit, drag.start_x, drag.start_y, drag.end_x, drag.end_y
            )
            permit.require_active()
            return _action("drag", drag.window_id, "Drag sent.")
        case ComputerScrollRequest() as scroll:
            permit = active.consume_observation(
                scroll.observation_id, scroll.window_id, ((scroll.x, scroll.y),)
            )
            active.platform.scroll(
                permit,
                scroll.x,
                scroll.y,
                scroll.delta_x,
                scroll.delta_y,
            )
            permit.require_active()
            return _action("scroll", scroll.window_id, "Scroll sent.")
        case ComputerTypeTextRequest() as typing:
            permit = active.consume_observation(typing.observation_id, typing.window_id)
            active.platform.type_text(permit, typing.text)
            permit.require_active()
            return _action("type_text", typing.window_id, "Text typed.")
        case ComputerPressKeysRequest() as keypress:
            permit = active.consume_observation(keypress.observation_id, keypress.window_id)
            active.platform.press_keys(permit, keypress.keys)
            permit.require_active()
            return _action("press_keys", keypress.window_id, "Keys pressed.")
        case ComputerCloseRequest() as close:
            permit = active.consume_observation(close.observation_id, close.window_id)
            active.platform.close(permit)
            permit.require_active()
            return _action("close", close.window_id, "Close requested.")
        case ComputerSetClipboardRequest() as clipboard:
            permit = active.consume_observation(
                clipboard.observation_id,
                clipboard.window_id,
            )
            active.platform.set_clipboard(permit, clipboard.text)
            permit.require_active()
            return _action("set_clipboard", clipboard.window_id, "Clipboard text set.")
        case ComputerStopRequest():
            raise ComputerControlError("Computer stop must use the controller stop path.")
    assert_never(request)


def _action(
    action: ComputerActionName,
    window_id: int | None,
    message: str,
) -> ComputerActionOutput:
    LOGGER.info("remote_computer_action action=%s window_id=%s", action, window_id)
    return ComputerActionOutput(action=action, window_id=window_id, message=message)

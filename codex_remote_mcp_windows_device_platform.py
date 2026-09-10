from __future__ import annotations

from collections.abc import Callable
from typing import Final, final

from codex_remote_mcp_computer_contracts import ComputerActionPermit, ComputerCapture
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_device_input import (
    click_device_window,
    drag_device_window,
    press_device_keys,
    scroll_device_window,
    type_device_text,
)
from codex_remote_mcp_windows_input import set_clipboard_text
from codex_remote_mcp_windows_native import USER32
from codex_remote_mcp_windows_platform import WindowsComputerPlatform
from codex_remote_mcp_windows_screenshot import capture_device_window
from codex_remote_mcp_windows_device_windows import (
    activate_device_window,
    list_device_windows,
    require_matching_active_device_window,
    resolve_device_window,
)
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry

WM_CLOSE: Final = 0x0010


@final
class WindowsDeviceComputerPlatform:
    def __init__(self) -> None:
        self._launched = WindowsComputerPlatform()

    def list_windows(self) -> tuple[ComputerWindowEntry, ...]:
        return list_device_windows()

    def stop(self, *, deadline_monotonic: float | None = None) -> None:
        self._launched.stop(deadline_monotonic=deadline_monotonic)

    def screenshot(self, window_id: int) -> ComputerCapture:
        return capture_device_window(resolve_device_window(window_id))

    def activate(self, window_id: int) -> ComputerWindowEntry:
        return activate_device_window(window_id)

    def launch(
        self,
        app: str,
        *,
        ensure_active: Callable[[], None] | None = None,
    ) -> None:
        self._launched.launch(app, ensure_active=ensure_active)

    def close(self, permit: ComputerActionPermit) -> None:
        permit.require_active()
        _ = require_matching_active_device_window(permit.identity)
        if not USER32.PostMessageW(permit.identity.window_id, WM_CLOSE, 0, 0):
            raise ComputerControlError(
                "Windows rejected the close request. An elevated or secure window may require direct user control."
            )

    def click(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        button: str,
        click_count: int,
    ) -> None:
        click_device_window(permit, x, y, button, click_count)

    def drag(
        self,
        permit: ComputerActionPermit,
        start_x: int,
        start_y: int,
        end_x: int,
        end_y: int,
    ) -> None:
        drag_device_window(permit, start_x, start_y, end_x, end_y)

    def scroll(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        delta_x: int,
        delta_y: int,
    ) -> None:
        scroll_device_window(permit, x, y, delta_x, delta_y)

    def type_text(self, permit: ComputerActionPermit, text: str) -> None:
        type_device_text(permit, text)

    def press_keys(self, permit: ComputerActionPermit, keys: tuple[str, ...]) -> None:
        press_device_keys(permit, keys)

    def set_clipboard(self, permit: ComputerActionPermit, text: str) -> None:
        permit.require_active()
        _ = require_matching_active_device_window(permit.identity)
        set_clipboard_text(text)
        permit.require_active()
        _ = require_matching_active_device_window(permit.identity)

from __future__ import annotations

from codex_remote_mcp_computer_contracts import ComputerCapture
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_capture_native import capture_window
from codex_remote_mcp_windows_windows import (
    ResolvedWindow,
    require_matching_active_window,
)

MAX_SCREENSHOT_BYTES = 8_500_000
MAX_CAPTURE_DIMENSION = 4_096
MAX_CAPTURE_PIXELS = 8_294_400


def capture_resolved_window(resolved: ResolvedWindow) -> ComputerCapture:
    window = resolved.entry
    if window.process_name.casefold() == "chrome.exe":
        raise ComputerControlError(
            "Chrome screenshots are unavailable because web content can include "
            + "unverifiable sign-in or secret surfaces."
        )
    active = require_matching_active_window(resolved.identity)
    window = active.entry
    if (
        window.width > MAX_CAPTURE_DIMENSION
        or window.height > MAX_CAPTURE_DIMENSION
        or window.width * window.height > MAX_CAPTURE_PIXELS
    ):
        raise ComputerControlError(
            "The window is too large to capture. Resize it and try again."
        )
    png = capture_window(window.window_id, window.width, window.height)
    _ = require_matching_active_window(active.identity)
    if len(png) > MAX_SCREENSHOT_BYTES:
        raise ComputerControlError(
            "The screenshot is too large to send. Resize the window and try again."
        )
    return ComputerCapture(window=window, identity=active.identity, png=png)


def capture_device_window(resolved: ResolvedWindow) -> ComputerCapture:
    from codex_remote_mcp_windows_device_windows import (
        require_matching_active_device_window,
    )

    active = require_matching_active_device_window(resolved.identity)
    window = active.entry
    if (
        window.width > MAX_CAPTURE_DIMENSION
        or window.height > MAX_CAPTURE_DIMENSION
        or window.width * window.height > MAX_CAPTURE_PIXELS
    ):
        raise ComputerControlError(
            "The window is too large to capture. Resize it and try again."
        )
    png = capture_window(window.window_id, window.width, window.height)
    _ = require_matching_active_device_window(active.identity)
    if len(png) > MAX_SCREENSHOT_BYTES:
        raise ComputerControlError(
            "The screenshot is too large to send. Resize the window and try again."
        )
    return ComputerCapture(window=window, identity=active.identity, png=png)

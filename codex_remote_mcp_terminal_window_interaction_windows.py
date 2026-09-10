from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import Final, final, override

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_runtime import inherited_terminal_environment
from codex_remote_mcp_terminal_window_identity_windows import (
    activate_terminal_window,
    require_terminal_window_rect,
)
from codex_remote_mcp_terminal_window_input_windows import (
    press_terminal_window_keys,
    type_terminal_window_text,
)
from codex_remote_mcp_terminal_window_interaction_types import (
    TerminalWindowCapture,
    TerminalWindowInteractionBackend,
    TerminalWindowObservation,
    terminal_window_identity_digest,
)
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from codex_remote_mcp_windows_capture_native import capture_window

_CREATE_NO_WINDOW: Final = 0x08000000
_MAX_CAPTURE_DIMENSION: Final = 4_096
_MAX_CAPTURE_PIXELS: Final = 8_294_400
_MAX_SCREENSHOT_BYTES: Final = 8_500_000


@final
class WindowsTerminalWindowInteractionBackend(TerminalWindowInteractionBackend):
    @override
    def require_supported(self) -> None:
        return None

    @override
    def capture(self, window: OwnedTerminalWindow) -> TerminalWindowCapture:
        rect = require_terminal_window_rect(window)
        if (
            rect.width > _MAX_CAPTURE_DIMENSION
            or rect.height > _MAX_CAPTURE_DIMENSION
            or rect.width * rect.height > _MAX_CAPTURE_PIXELS
        ):
            raise TerminalExecutionError("terminal window is too large to capture")
        try:
            png = capture_window(window.entry.window_id, rect.width, rect.height)
        except ComputerControlError as exc:
            raise TerminalExecutionError(exc.reason) from exc
        if len(png) > _MAX_SCREENSHOT_BYTES:
            raise TerminalExecutionError("terminal window screenshot is too large")
        if require_terminal_window_rect(window) != rect:
            raise TerminalExecutionError("terminal window changed during capture")
        return TerminalWindowCapture(rect=rect, png=png)

    @override
    def activate(self, window: OwnedTerminalWindow) -> bool:
        return activate_terminal_window(window)

    @override
    def type_text(self, window: OwnedTerminalWindow, text: str) -> bool:
        return type_terminal_window_text(window, text)

    @override
    def press_keys(
        self,
        window: OwnedTerminalWindow,
        keys: tuple[str, ...],
    ) -> tuple[bool, tuple[str, ...]]:
        return press_terminal_window_keys(window, keys)

    @override
    def interrupt(self, window: OwnedTerminalWindow) -> None:
        _ = require_terminal_window_rect(window)
        bootstrap_process_id = window.process.pid
        helper = Path(__file__).with_name("codex_remote_mcp_terminal_interrupt_helper.py")
        try:
            completed = subprocess.run(
                [sys.executable, "-I", "-S", str(helper), str(bootstrap_process_id)],
                check=False,
                timeout=5,
                env=inherited_terminal_environment(),
                creationflags=_CREATE_NO_WINDOW,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise TerminalExecutionError("terminal interrupt helper failed") from exc
        if completed.returncode != 0:
            raise TerminalExecutionError(
                f"terminal interrupt helper failed (exit {completed.returncode})"
            )

    @override
    def matches_observation(
        self,
        window: OwnedTerminalWindow,
        observation: TerminalWindowObservation,
    ) -> bool:
        try:
            rect = require_terminal_window_rect(window)
        except TerminalExecutionError:
            return False
        return (
            observation.terminal_window_id == window.entry.terminal_window_id
            and observation.identity_digest == terminal_window_identity_digest(window)
            and observation.window_process_id == window.window_process_id
            and observation.rect == rect
        )


__all__ = ["WindowsTerminalWindowInteractionBackend"]

from __future__ import annotations

# pyright: reportAny=false

import ctypes
import ctypes.wintypes as wt
from pathlib import Path

from codex_remote_mcp_redaction import redact
from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from codex_remote_mcp_windows_native import USER32, Rect
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowEntry,
    TerminalWindowShell,
)


def inspect_owned_terminal_window(
    window: OwnedTerminalWindow,
) -> TerminalWindowEntry | None:
    if _owned_window_is_stale(window):
        return None
    try:
        entry = terminal_window_entry(
            window.entry.terminal_window_id,
            window.entry.window_id,
            window.process.pid,
            window.entry.shell,
            Path(window.entry.cwd),
        )
    except TerminalExecutionError:
        if _owned_window_is_stale(window):
            return None
        raise
    if _owned_window_is_stale(window):
        return None
    return entry


def terminal_window_entry(
    terminal_window_id: str,
    window_id: int,
    process_id: int,
    shell: TerminalWindowShell,
    cwd: Path,
) -> TerminalWindowEntry:
    if not USER32.IsWindow(window_id):
        raise TerminalExecutionError("terminal window is no longer available")
    rect = Rect()
    if not USER32.GetWindowRect(window_id, ctypes.byref(rect)):
        raise TerminalExecutionError("Windows could not inspect the terminal window")
    title = redact(window_title(window_id))[:500]
    if not title:
        raise TerminalExecutionError("terminal window has no usable title")
    return TerminalWindowEntry(
        terminal_window_id=terminal_window_id,
        window_id=window_id,
        process_id=process_id,
        shell=shell,
        cwd=str(cwd),
        title=title,
    )


def require_window_process_id(window_id: int) -> int:
    owner = wt.DWORD()
    thread_id = int(USER32.GetWindowThreadProcessId(window_id, ctypes.byref(owner)))
    if thread_id <= 0 or owner.value <= 0:
        raise TerminalExecutionError("Windows could not identify the terminal window")
    return int(owner.value)


def try_window_process_id(window_id: int) -> int | None:
    try:
        return require_window_process_id(window_id)
    except TerminalExecutionError:
        if not USER32.IsWindow(window_id):
            return None
        raise


def _owned_window_is_stale(window: OwnedTerminalWindow) -> bool:
    try:
        if window.process.poll() is not None:
            return True
    except OSError as exc:
        raise TerminalExecutionError(
            "Windows could not inspect the terminal window process"
        ) from exc
    if not USER32.IsWindow(window.entry.window_id):
        return True
    current_process_id = try_window_process_id(window.entry.window_id)
    return current_process_id is None or (
        window.window_process_id is not None
        and current_process_id != window.window_process_id
    )


def window_title(window_id: int) -> str:
    length = int(USER32.GetWindowTextLengthW(window_id))
    if length <= 0:
        return ""
    buffer = ctypes.create_unicode_buffer(length + 1)
    USER32.GetWindowTextW(window_id, buffer, length + 1)
    return buffer.value


__all__ = [
    "inspect_owned_terminal_window",
    "require_window_process_id",
    "terminal_window_entry",
    "try_window_process_id",
    "window_title",
]

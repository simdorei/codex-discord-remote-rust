# pyright: reportAny=false
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import hashlib
import time
from pathlib import Path
from typing import Final

from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_native import (
    KERNEL32,
    USER32,
    EnumWindowsCallback,
    Rect,
)
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry

PROCESS_QUERY_LIMITED_INFORMATION: Final = 0x1000
SW_RESTORE: Final = 9


def list_device_windows() -> tuple[ComputerWindowEntry, ...]:
    entries: list[ComputerWindowEntry] = []

    @EnumWindowsCallback
    def visit(hwnd: int, _lparam: int) -> bool:
        try:
            entry = resolve_device_window(int(hwnd)).entry
        except ComputerControlError:
            return True
        if entry.width >= 80 and entry.height >= 60:
            entries.append(entry)
        return True

    if not USER32.EnumWindows(visit, 0):
        raise ComputerControlError("Windows could not enumerate visible windows.")
    return tuple(entries)


def resolve_device_window(window_id: int) -> ResolvedWindow:
    if not USER32.IsWindow(window_id) or not USER32.IsWindowVisible(window_id):
        raise ComputerControlError("The selected window is no longer visible.")
    title = _window_title(window_id)
    if not title:
        raise ComputerControlError("The selected window has no usable title.")
    process_id, process_path = _process_identity(window_id)
    rect = Rect()
    if not USER32.GetWindowRect(window_id, ctypes.byref(rect)):
        raise ComputerControlError("Windows could not read the selected window bounds.")
    width = int(rect.right - rect.left)
    height = int(rect.bottom - rect.top)
    if width <= 0 or height <= 0:
        raise ComputerControlError("The selected window has invalid bounds.")
    entry = ComputerWindowEntry(
        window_id=window_id,
        title=title[:512],
        process_name=Path(process_path).name,
        left=int(rect.left),
        top=int(rect.top),
        width=width,
        height=height,
        active=window_id == int(USER32.GetForegroundWindow()),
    )
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=window_id,
            process_id=process_id,
            process_path=process_path.casefold(),
            title_digest=hashlib.sha256(title.encode("utf-8")).hexdigest(),
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
        ),
    )


def activate_device_window(window_id: int) -> ComputerWindowEntry:
    _ = resolve_device_window(window_id)
    USER32.ShowWindow(window_id, SW_RESTORE)
    _set_foreground(window_id)
    time.sleep(0.12)
    entry = resolve_device_window(window_id).entry
    if int(USER32.GetForegroundWindow()) != window_id:
        raise ComputerControlError("Windows did not allow the window to become active.")
    return entry.model_copy(update={"active": True})


def require_matching_device_window(
    identity: ComputerWindowIdentity,
) -> ResolvedWindow:
    current = resolve_device_window(identity.window_id)
    if not _same_window(identity, current.identity):
        raise ComputerControlError(
            "The window changed after the screenshot. Take a fresh screenshot."
        )
    return current


def require_matching_active_device_window(
    identity: ComputerWindowIdentity,
) -> ResolvedWindow:
    return _require_active(
        require_matching_device_window(identity),
        identity.window_id,
    )


def require_same_active_device_window_after_mutation(
    identity: ComputerWindowIdentity,
) -> ResolvedWindow:
    current = resolve_device_window(identity.window_id)
    if not _same_window_frame(identity, current.identity):
        raise ComputerControlError(
            "The window changed after the screenshot. Take a fresh screenshot."
        )
    return _require_active(current, identity.window_id)


def _set_foreground(window_id: int) -> None:
    if USER32.SetForegroundWindow(window_id):
        return
    foreground = int(USER32.GetForegroundWindow())
    foreground_thread = int(USER32.GetWindowThreadProcessId(foreground, None))
    current_thread = int(KERNEL32.GetCurrentThreadId())
    attached = (
        foreground_thread != 0
        and foreground_thread != current_thread
        and bool(USER32.AttachThreadInput(current_thread, foreground_thread, True))
    )
    try:
        USER32.BringWindowToTop(window_id)
        USER32.SetForegroundWindow(window_id)
    finally:
        if attached:
            USER32.AttachThreadInput(current_thread, foreground_thread, False)


def _require_active(current: ResolvedWindow, window_id: int) -> ResolvedWindow:
    if int(USER32.GetForegroundWindow()) != window_id:
        raise ComputerControlError(
            "The active window changed. Take a fresh screenshot before continuing."
        )
    return current


def _same_window(
    expected: ComputerWindowIdentity,
    current: ComputerWindowIdentity,
) -> bool:
    return _same_window_frame(expected, current) and (
        expected.title_digest == current.title_digest
    )


def _same_window_frame(
    expected: ComputerWindowIdentity,
    current: ComputerWindowIdentity,
) -> bool:
    return (
        expected.window_id == current.window_id
        and expected.process_id == current.process_id
        and expected.process_path == current.process_path
        and expected.left == current.left
        and expected.top == current.top
        and expected.width == current.width
        and expected.height == current.height
    )


def _window_title(window_id: int) -> str:
    length = int(USER32.GetWindowTextLengthW(window_id))
    if length <= 0:
        return ""
    buffer = ctypes.create_unicode_buffer(length + 1)
    USER32.GetWindowTextW(window_id, buffer, length + 1)
    return " ".join(buffer.value.split())


def _process_identity(window_id: int) -> tuple[int, str]:
    process_id = wt.DWORD()
    USER32.GetWindowThreadProcessId(window_id, ctypes.byref(process_id))
    return int(process_id.value), _process_path(int(process_id.value))


def _process_path(process_id: int) -> str:
    handle = KERNEL32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, process_id)
    if not handle:
        raise ComputerControlError("Windows denied process identity inspection.")
    try:
        size = wt.DWORD(32_768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not KERNEL32.QueryFullProcessImageNameW(
            handle,
            0,
            buffer,
            ctypes.byref(size),
        ):
            raise ComputerControlError("Windows could not identify the application.")
        return str(Path(buffer.value))
    finally:
        KERNEL32.CloseHandle(handle)

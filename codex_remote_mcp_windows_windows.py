# pyright: reportAny=false
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import hashlib
import time
from dataclasses import dataclass
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
from codex_remote_mcp_windows_policy import require_allowed_window
from codex_remote_mcp_windows_text import read_control_text
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry

PROCESS_QUERY_LIMITED_INFORMATION: Final = 0x1000
SW_RESTORE: Final = 9
MAX_NOTEPAD_IDENTITY_CHARS: Final = 1_048_576


@dataclass(frozen=True, slots=True)
class ResolvedWindow:
    entry: ComputerWindowEntry
    identity: ComputerWindowIdentity


def list_allowed_windows() -> tuple[ComputerWindowEntry, ...]:
    entries: list[ComputerWindowEntry] = []

    @EnumWindowsCallback
    def visit(hwnd: int, _lparam: int) -> bool:
        try:
            entry = resolve_allowed_window(int(hwnd)).entry
        except ComputerControlError:
            return True
        if entry.width >= 80 and entry.height >= 60:
            entries.append(entry)
        return True

    if not USER32.EnumWindows(visit, 0):
        raise ComputerControlError("Windows could not enumerate visible windows.")
    return tuple(entries)


def resolve_allowed_window(window_id: int) -> ResolvedWindow:
    if not USER32.IsWindow(window_id) or not USER32.IsWindowVisible(window_id):
        raise ComputerControlError("The selected window is no longer visible.")
    title = _window_title(window_id)
    if not title:
        raise ComputerControlError("The selected window has no usable title.")
    process_id, process_path = _process_identity(window_id)
    safe_title, process_name = require_allowed_window(process_path, title)
    rect = Rect()
    if not USER32.GetWindowRect(window_id, ctypes.byref(rect)):
        raise ComputerControlError("Windows could not read the selected window bounds.")
    width = int(rect.right - rect.left)
    height = int(rect.bottom - rect.top)
    if width <= 0 or height <= 0:
        raise ComputerControlError("The selected window has invalid bounds.")
    entry = ComputerWindowEntry(
        window_id=window_id,
        title=safe_title,
        process_name=process_name,
        left=int(rect.left),
        top=int(rect.top),
        width=width,
        height=height,
        active=window_id == int(USER32.GetForegroundWindow()),
    )
    surface_window_id = None
    surface_digest = ""
    surface_rect = Rect()
    if process_name.casefold() == "notepad.exe":
        surface_window_id, surface_rect, surface_digest = _notepad_surface(window_id)
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=window_id,
            process_id=process_id,
            process_path=process_path.casefold(),
            title_digest=_title_identity_digest(title, process_name),
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
            surface_window_id=surface_window_id,
            surface_digest=surface_digest,
            surface_left=int(surface_rect.left),
            surface_top=int(surface_rect.top),
            surface_width=int(surface_rect.right - surface_rect.left),
            surface_height=int(surface_rect.bottom - surface_rect.top),
        ),
    )


def activate_window(window_id: int) -> ComputerWindowEntry:
    _ = resolve_allowed_window(window_id)
    USER32.ShowWindow(window_id, SW_RESTORE)
    _set_foreground(window_id)
    time.sleep(0.12)
    entry = resolve_allowed_window(window_id).entry
    if int(USER32.GetForegroundWindow()) != window_id:
        raise ComputerControlError("Windows did not allow the window to become active.")
    return entry.model_copy(update={"active": True})


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


def require_matching_window(identity: ComputerWindowIdentity) -> ResolvedWindow:
    current = resolve_allowed_window(identity.window_id)
    if not _same_window(identity, current.identity):
        raise ComputerControlError(
            "The window changed after the screenshot. Take a fresh screenshot."
        )
    return current


def require_matching_active_window(
    identity: ComputerWindowIdentity,
) -> ResolvedWindow:
    return _require_active_window(require_matching_window(identity), identity.window_id)


def require_same_active_window_after_mutation(
    identity: ComputerWindowIdentity,
) -> ResolvedWindow:
    current = resolve_allowed_window(identity.window_id)
    if not _same_window_frame(identity, current.identity):
        raise ComputerControlError(
            "The window changed after the screenshot. Take a fresh screenshot."
        )
    return _require_active_window(current, identity.window_id)


def _require_active_window(current: ResolvedWindow, window_id: int) -> ResolvedWindow:
    if int(USER32.GetForegroundWindow()) != window_id:
        raise ComputerControlError(
            "The active window changed. Take a fresh screenshot before continuing."
        )
    return current


def _same_window(
    expected: ComputerWindowIdentity,
    current: ComputerWindowIdentity,
) -> bool:
    return (
        _same_window_frame(expected, current)
        and expected.title_digest == current.title_digest
        and expected.surface_digest == current.surface_digest
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
        and expected.surface_window_id == current.surface_window_id
        and expected.surface_left == current.surface_left
        and expected.surface_top == current.surface_top
        and expected.surface_width == current.surface_width
        and expected.surface_height == current.surface_height
    )


def _title_identity_digest(title: str, process_name: str) -> str:
    _ = process_name
    return hashlib.sha256(title.encode("utf-8")).hexdigest()


def _notepad_surface(window_id: int) -> tuple[int, Rect, str]:
    candidates: list[int] = []

    @EnumWindowsCallback
    def visit(child: int, _lparam: int) -> bool:
        if (
            USER32.IsWindowVisible(child)
            and _window_class(int(child)).casefold() == "edit"
        ):
            candidates.append(int(child))
        return True

    if not USER32.EnumChildWindows(window_id, visit, 0):
        raise ComputerControlError("Windows could not inspect the Notepad editor.")
    if len(candidates) != 1:
        raise ComputerControlError(
            "Only a single classic Notepad document can be controlled safely."
        )
    surface = candidates[0]
    rect = Rect()
    if not USER32.GetWindowRect(surface, ctypes.byref(rect)):
        raise ComputerControlError("Windows could not read the Notepad editor bounds.")
    content = read_control_text(surface, limit=MAX_NOTEPAD_IDENTITY_CHARS)
    material = f"{surface}\0Edit\0{content}".encode()
    return surface, rect, hashlib.sha256(material).hexdigest()


def _window_class(window_id: int) -> str:
    buffer = ctypes.create_unicode_buffer(256)
    if USER32.GetClassNameW(window_id, buffer, len(buffer)) <= 0:
        raise ComputerControlError("Windows could not identify the window control.")
    return buffer.value


def window_is_visible(window_id: int) -> bool:
    return bool(USER32.IsWindow(window_id) and USER32.IsWindowVisible(window_id))


def _window_text(
    window_id: int,
    *,
    limit: int | None = None,
    normalize_whitespace: bool = False,
) -> str:
    length = int(USER32.GetWindowTextLengthW(window_id))
    if length <= 0:
        return ""
    if limit is not None and length > limit:
        raise ComputerControlError(
            "The Notepad document is too large to control safely."
        )
    buffer = ctypes.create_unicode_buffer(length + 1)
    USER32.GetWindowTextW(window_id, buffer, length + 1)
    if normalize_whitespace:
        return " ".join(buffer.value.split())
    return buffer.value


def _window_title(window_id: int) -> str:
    return _window_text(window_id, normalize_whitespace=True)


def _process_identity(window_id: int) -> tuple[int, str]:
    process_id = wt.DWORD()
    USER32.GetWindowThreadProcessId(window_id, ctypes.byref(process_id))
    return int(process_id.value), _process_path(int(process_id.value))


def process_matches(process_id: int, expected_path: str) -> bool:
    return _process_path(process_id).casefold() == expected_path.casefold()


def _process_path(process_id: int) -> str:
    handle = KERNEL32.OpenProcess(
        PROCESS_QUERY_LIMITED_INFORMATION,
        False,
        process_id,
    )
    if not handle:
        raise ComputerControlError("Windows denied process identity inspection.")
    try:
        size = wt.DWORD(32_768)
        buffer = ctypes.create_unicode_buffer(size.value)
        if not KERNEL32.QueryFullProcessImageNameW(
            handle, 0, buffer, ctypes.byref(size)
        ):
            raise ComputerControlError("Windows could not identify the application.")
        return str(Path(buffer.value))
    finally:
        KERNEL32.CloseHandle(handle)

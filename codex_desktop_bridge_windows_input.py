# pyright: reportAny=false, reportUnannotatedClassAttribute=false, reportUnknownMemberType=false
from __future__ import annotations

import ctypes
import time
from collections.abc import Callable

from codex_desktop_bridge_windows_native import (
    CF_UNICODETEXT,
    GMEM_MOVEABLE,
    INPUT,
    INPUT_KEYBOARD,
    INPUT_UNION,
    KEYBDINPUT,
    KEYEVENTF_KEYUP,
    RECT,
    EnumWindowsProc,
    kernel32,
    user32,
)
from codex_thread_models import WindowInfo

class WindowsInputError(RuntimeError):
    pass


class ClipboardOpenError(WindowsInputError):
    pass


class ClipboardEmptyError(WindowsInputError):
    pass


class ClipboardAllocationError(WindowsInputError):
    pass


class ClipboardLockError(WindowsInputError):
    pass


class ClipboardSetDataError(WindowsInputError):
    pass


WindowCallback = Callable[[int], bool]


def get_window_text_length(hwnd: int) -> int:
    return int(user32.GetWindowTextLengthW(hwnd))


def read_window_text(hwnd: int, max_count: int) -> str:
    buffer = ctypes.create_unicode_buffer(max_count)
    user32.GetWindowTextW(hwnd, buffer, max_count)
    return buffer.value


def enum_windows(callback: WindowCallback) -> None:
    @EnumWindowsProc
    def enum_windows_proc(hwnd: int, _lparam: int) -> bool:
        return bool(callback(int(hwnd)))

    user32.EnumWindows(enum_windows_proc, 0)


def get_window_rect_tuple(hwnd: int) -> tuple[int, int, int, int] | None:
    rect = RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return None
    return (rect.left, rect.top, rect.right, rect.bottom)


def is_window_visible(hwnd: int) -> bool:
    return bool(user32.IsWindowVisible(hwnd))


def get_foreground_window() -> int:
    return int(user32.GetForegroundWindow())


def show_window(hwnd: int, command: int) -> None:
    user32.ShowWindow(hwnd, command)


def set_foreground_window(hwnd: int) -> None:
    user32.SetForegroundWindow(hwnd)


def bring_window_to_top(hwnd: int) -> None:
    user32.BringWindowToTop(hwnd)


def get_clipboard_text() -> str | None:
    if not user32.OpenClipboard(None):
        return None
    try:
        handle = user32.GetClipboardData(CF_UNICODETEXT)
        if not handle:
            return ""
        pointer = kernel32.GlobalLock(handle)
        if not pointer:
            return ""
        try:
            return ctypes.wstring_at(pointer)
        finally:
            kernel32.GlobalUnlock(handle)
    finally:
        user32.CloseClipboard()


def set_clipboard_text(text: str) -> None:
    if not user32.OpenClipboard(None):
        raise ClipboardOpenError("Failed to open the clipboard.")
    try:
        if not user32.EmptyClipboard():
            raise ClipboardEmptyError("Failed to empty the clipboard.")

        data = text.encode("utf-16-le") + b"\x00\x00"
        h_global = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
        if not h_global:
            raise ClipboardAllocationError("GlobalAlloc failed.")

        pointer = kernel32.GlobalLock(h_global)
        if not pointer:
            kernel32.GlobalFree(h_global)
            raise ClipboardLockError("GlobalLock failed.")

        try:
            _ = ctypes.memmove(pointer, data, len(data))
        finally:
            kernel32.GlobalUnlock(h_global)

        if not user32.SetClipboardData(CF_UNICODETEXT, h_global):
            kernel32.GlobalFree(h_global)
            raise ClipboardSetDataError("SetClipboardData failed.")
    finally:
        user32.CloseClipboard()


def send_key_event(vk: int, keyup: bool = False) -> None:
    flags = KEYEVENTF_KEYUP if keyup else 0
    input_struct = INPUT(
        type=INPUT_KEYBOARD,
        union=INPUT_UNION(
            ki=KEYBDINPUT(
                wVk=vk,
                wScan=0,
                dwFlags=flags,
                time=0,
                dwExtraInfo=0,
            )
        ),
    )
    user32.SendInput(1, ctypes.byref(input_struct), ctypes.sizeof(INPUT))


def send_hotkey(*keys: int) -> None:
    for vk in keys:
        send_key_event(vk, keyup=False)
    for vk in reversed(keys):
        send_key_event(vk, keyup=True)
    time.sleep(0.05)


def click_window(window: WindowInfo, x_ratio: float, y_offset: int) -> tuple[int, int]:
    x = window.left + int(window.width * x_ratio)
    y = max(window.top + 40, window.bottom - y_offset)
    user32.SetCursorPos(x, y)
    time.sleep(0.1)
    user32.mouse_event(0x0002, 0, 0, 0, 0)
    user32.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.1)
    return x, y

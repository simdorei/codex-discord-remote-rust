# pyright: reportAny=false
from __future__ import annotations

import ctypes
from typing import Final

from codex_remote_mcp_computer_contracts import ComputerActionPermit
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_native import KERNEL32, USER32
from codex_remote_mcp_windows_policy import (
    parse_key_chord,
    require_allowed_interaction,
)
from codex_remote_mcp_windows_windows import (
    ResolvedWindow,
    require_matching_active_window,
    require_same_active_window_after_mutation,
)

CF_UNICODETEXT: Final = 13
GMEM_MOVEABLE: Final = 0x0002
SMTO_ABORTIFHUNG: Final = 0x0002
MESSAGE_TIMEOUT_MS: Final = 1_000
WM_KEYDOWN: Final = 0x0100
WM_KEYUP: Final = 0x0101
WM_MOUSEMOVE: Final = 0x0200
WM_LBUTTONDOWN: Final = 0x0201
WM_LBUTTONUP: Final = 0x0202
WM_LBUTTONDBLCLK: Final = 0x0203
WM_CUT: Final = 0x0300
WM_COPY: Final = 0x0301
WM_UNDO: Final = 0x0304
EM_SETSEL: Final = 0x00B1
EM_REPLACESEL: Final = 0x00C2
EM_LINESCROLL: Final = 0x00B6
MK_LBUTTON: Final = 0x0001
MUTATING_SINGLE_KEYS: Final = frozenset(
    {"BACKSPACE", "DELETE", "ENTER", "SPACE", "TAB"}
)


def click_window(
    permit: ComputerActionPermit,
    x: int,
    y: int,
    button: str,
    click_count: int,
) -> None:
    if button != "left":
        raise ComputerControlError("Only a window-bound left click is available.")
    surface, points = _notepad_target(permit, "click", ((x, y),))
    position = _pack_point(*points[0])
    _ = _recheck_post_input(permit, content_may_change=False)
    for index in range(click_count):
        down = WM_LBUTTONDBLCLK if index else WM_LBUTTONDOWN
        _ = _send_message(surface, down, MK_LBUTTON, position)
        _ = _send_message(surface, WM_LBUTTONUP, 0, position)
    _ = _recheck_post_input(permit, content_may_change=False)


def drag_window(
    permit: ComputerActionPermit,
    start_x: int,
    start_y: int,
    end_x: int,
    end_y: int,
) -> None:
    surface, points = _notepad_target(
        permit,
        "drag",
        ((start_x, start_y), (end_x, end_y)),
    )
    start, end = (_pack_point(*point) for point in points)
    _ = _recheck_post_input(permit, content_may_change=False)
    _ = _send_message(surface, WM_LBUTTONDOWN, MK_LBUTTON, start)
    _ = _send_message(surface, WM_MOUSEMOVE, MK_LBUTTON, end)
    _ = _send_message(surface, WM_LBUTTONUP, 0, end)
    _ = _recheck_post_input(permit, content_may_change=False)


def scroll_window(
    permit: ComputerActionPermit,
    x: int,
    y: int,
    delta_x: int,
    delta_y: int,
) -> None:
    require_allowed_interaction(permit.identity.process_path, "scroll")
    if not delta_x and not delta_y:
        raise ComputerControlError("A non-zero scroll amount is required.")
    current = _recheck_active(permit)
    _validate_point(current.entry.width, current.entry.height, x, y)
    if current.entry.process_name.casefold() != "notepad.exe":
        raise ComputerControlError("This interaction is available only in Notepad.")
    surface = _required_surface(permit)
    horizontal = _wheel_steps(delta_x)
    vertical = -_wheel_steps(delta_y)
    _ = _recheck_active(permit)
    _ = _send_message(surface, EM_LINESCROLL, horizontal, vertical)
    _ = _recheck_active(permit)


def type_window_text(permit: ComputerActionPermit, text: str) -> None:
    surface, _ = _notepad_target(permit, "type_text")
    buffer = ctypes.create_unicode_buffer(text)
    pointer = ctypes.cast(buffer, ctypes.c_void_p).value
    if pointer is None:
        raise ComputerControlError("Windows could not prepare the text input.")
    _ = _recheck_active(permit)
    _ = _send_message(surface, EM_REPLACESEL, 1, pointer)
    _ = _recheck_post_input(permit, content_may_change=True)


def press_window_keys(permit: ComputerActionPermit, keys: tuple[str, ...]) -> None:
    codes = parse_key_chord(keys)
    surface, _ = _notepad_target(permit, "press_keys")
    normalized = frozenset(key.strip().upper().replace(" ", "") for key in keys)
    edit_messages = {
        frozenset({"CTRL", "C"}): WM_COPY,
        frozenset({"CTRL", "X"}): WM_CUT,
        frozenset({"CTRL", "Z"}): WM_UNDO,
    }
    if normalized == frozenset({"CTRL", "A"}):
        _ = _recheck_active(permit)
        _ = _send_message(surface, EM_SETSEL, 0, -1)
        _ = _recheck_post_input(permit, content_may_change=False)
        return
    if message := edit_messages.get(normalized):
        _ = _recheck_active(permit)
        _ = _send_message(surface, message)
        _ = _recheck_post_input(
            permit,
            content_may_change=normalized != frozenset({"CTRL", "C"}),
        )
        return
    if len(codes) != 1:
        raise ComputerControlError("This protected keyboard shortcut is not available.")
    _ = _recheck_active(permit)
    _ = _send_message(surface, WM_KEYDOWN, codes[0])
    _ = _send_message(surface, WM_KEYUP, codes[0])
    _ = _recheck_post_input(
        permit,
        content_may_change=bool(normalized & MUTATING_SINGLE_KEYS),
    )


def set_clipboard_text(text: str) -> None:
    if not USER32.OpenClipboard(0):
        raise ComputerControlError("Windows could not open the clipboard.")
    handle = 0
    try:
        if not USER32.EmptyClipboard():
            raise ComputerControlError("Windows could not clear the clipboard.")
        data = text.encode("utf-16-le") + b"\x00\x00"
        handle = KERNEL32.GlobalAlloc(GMEM_MOVEABLE, len(data))
        if not handle:
            raise ComputerControlError("Windows could not allocate clipboard memory.")
        pointer = KERNEL32.GlobalLock(handle)
        if not pointer:
            raise ComputerControlError("Windows could not lock clipboard memory.")
        try:
            _ = ctypes.memmove(pointer, data, len(data))
        finally:
            KERNEL32.GlobalUnlock(handle)
        if not USER32.SetClipboardData(CF_UNICODETEXT, handle):
            raise ComputerControlError("Windows could not set clipboard text.")
        handle = 0
    finally:
        if handle:
            KERNEL32.GlobalFree(handle)
        USER32.CloseClipboard()


def _notepad_target(
    permit: ComputerActionPermit,
    interaction: str,
    points: tuple[tuple[int, int], ...] = (),
) -> tuple[int, tuple[tuple[int, int], ...]]:
    require_allowed_interaction(permit.identity.process_path, interaction)
    current = _recheck_active(permit)
    if current.entry.process_name.casefold() != "notepad.exe":
        raise ComputerControlError("This interaction is available only in Notepad.")
    surface = _required_surface(permit)
    translated: list[tuple[int, int]] = []
    for x, y in points:
        _validate_point(current.entry.width, current.entry.height, x, y)
        surface_x = current.entry.left + x - permit.identity.surface_left
        surface_y = current.entry.top + y - permit.identity.surface_top
        _validate_point(
            permit.identity.surface_width,
            permit.identity.surface_height,
            surface_x,
            surface_y,
        )
        translated.append((surface_x, surface_y))
    return surface, tuple(translated)


def _recheck_active(permit: ComputerActionPermit) -> ResolvedWindow:
    permit.require_active()
    return require_matching_active_window(permit.identity)


def _recheck_post_input(
    permit: ComputerActionPermit,
    *,
    content_may_change: bool,
) -> ResolvedWindow:
    permit.require_active()
    if content_may_change:
        return require_same_active_window_after_mutation(permit.identity)
    return require_matching_active_window(permit.identity)


def _required_surface(permit: ComputerActionPermit) -> int:
    surface = permit.identity.surface_window_id
    if surface is None or permit.identity.surface_width <= 0:
        raise ComputerControlError(
            "Windows could not verify the Notepad editor surface."
        )
    return surface


def _send_message(
    window_id: int,
    message: int,
    wparam: int = 0,
    lparam: int = 0,
) -> int:
    result = ctypes.c_size_t()
    sent = USER32.SendMessageTimeoutW(
        window_id,
        message,
        wparam,
        lparam,
        SMTO_ABORTIFHUNG,
        MESSAGE_TIMEOUT_MS,
        ctypes.byref(result),
    )
    if not sent:
        raise ComputerControlError("The application did not accept window-bound input.")
    return int(result.value)


def _wheel_steps(delta: int) -> int:
    if delta == 0:
        return 0
    return delta // 120 if abs(delta) >= 120 else (1 if delta > 0 else -1)


def _pack_point(x: int, y: int) -> int:
    return (x & 0xFFFF) | ((y & 0xFFFF) << 16)


def _validate_point(width: int, height: int, x: int, y: int) -> None:
    if x < 0 or y < 0 or x >= width or y >= height:
        raise ComputerControlError("The requested point is outside the current window.")

# pyright: reportAny=false
from __future__ import annotations

import ctypes
from typing import Final

from codex_remote_mcp_computer_contracts import ComputerActionPermit
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_native import (
    USER32,
    Input,
    InputValue,
    KeyboardInput,
)
from codex_remote_mcp_windows_device_windows import (
    require_matching_active_device_window,
    require_same_active_device_window_after_mutation,
)
from codex_remote_mcp_windows_windows import ResolvedWindow

INPUT_KEYBOARD: Final = 1
KEYEVENTF_KEYUP: Final = 0x0002
KEYEVENTF_UNICODE: Final = 0x0004
MOUSEEVENTF_LEFTDOWN: Final = 0x0002
MOUSEEVENTF_LEFTUP: Final = 0x0004
MOUSEEVENTF_RIGHTDOWN: Final = 0x0008
MOUSEEVENTF_RIGHTUP: Final = 0x0010
MOUSEEVENTF_MIDDLEDOWN: Final = 0x0020
MOUSEEVENTF_MIDDLEUP: Final = 0x0040
MOUSEEVENTF_WHEEL: Final = 0x0800
MOUSEEVENTF_HWHEEL: Final = 0x1000
KEY_CODES: Final = {
    "BACKSPACE": 0x08,
    "TAB": 0x09,
    "ENTER": 0x0D,
    "SHIFT": 0x10,
    "CTRL": 0x11,
    "ALT": 0x12,
    "PAUSE": 0x13,
    "CAPSLOCK": 0x14,
    "ESC": 0x1B,
    "SPACE": 0x20,
    "PAGEUP": 0x21,
    "PAGEDOWN": 0x22,
    "END": 0x23,
    "HOME": 0x24,
    "LEFT": 0x25,
    "UP": 0x26,
    "RIGHT": 0x27,
    "DOWN": 0x28,
    "PRINTSCREEN": 0x2C,
    "INSERT": 0x2D,
    "DELETE": 0x2E,
    "WIN": 0x5B,
}
BUTTON_FLAGS: Final = {
    "left": (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
    "right": (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
    "middle": (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
}


def click_device_window(
    permit: ComputerActionPermit,
    x: int,
    y: int,
    button: str,
    click_count: int,
) -> None:
    _ = _move_to_window_point(permit, x, y)
    try:
        down, up = BUTTON_FLAGS[button]
    except KeyError as exc:
        raise ComputerControlError(f"Unsupported mouse button: {button}") from exc
    for _ in range(click_count):
        USER32.mouse_event(down, 0, 0, 0, 0)
        USER32.mouse_event(up, 0, 0, 0, 0)
    _ = _require_same_frame(permit)


def drag_device_window(
    permit: ComputerActionPermit,
    start_x: int,
    start_y: int,
    end_x: int,
    end_y: int,
) -> None:
    current = _move_to_window_point(permit, start_x, start_y)
    _validate_point(current, end_x, end_y)
    USER32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    try:
        _set_cursor(current.entry.left + end_x, current.entry.top + end_y)
    finally:
        USER32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    _ = _require_same_frame(permit)


def scroll_device_window(
    permit: ComputerActionPermit,
    x: int,
    y: int,
    delta_x: int,
    delta_y: int,
) -> None:
    _ = _move_to_window_point(permit, x, y)
    if delta_y:
        USER32.mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta_y & 0xFFFFFFFF, 0)
    if delta_x:
        USER32.mouse_event(MOUSEEVENTF_HWHEEL, 0, 0, delta_x & 0xFFFFFFFF, 0)
    _ = _require_same_frame(permit)


def type_device_text(permit: ComputerActionPermit, text: str) -> None:
    _ = _require_active(permit)
    units = tuple(
        int.from_bytes(encoded[index : index + 2], "little")
        for encoded in (text.encode("utf-16-le"),)
        for index in range(0, len(encoded), 2)
    )
    events = tuple(
        event
        for unit in units
        for event in (
            _keyboard_input(0, unit, KEYEVENTF_UNICODE),
            _keyboard_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
        )
    )
    _send_inputs(events)
    _ = _require_same_frame_after_mutation(permit)


def press_device_keys(permit: ComputerActionPermit, keys: tuple[str, ...]) -> None:
    _ = _require_active(permit)
    normalized = tuple(_normalize_key(key) for key in keys)
    if len(normalized) != len(set(normalized)):
        raise ComputerControlError("Duplicate keyboard keys are not available.")
    if frozenset(normalized) == frozenset({"CTRL", "ALT", "DELETE"}):
        raise ComputerControlError(
            "Ctrl+Alt+Delete is a Windows secure-screen action and requires the user."
        )
    codes = tuple(_key_code(key) for key in normalized)
    events = tuple(
        [_keyboard_input(code, 0, 0) for code in codes]
        + [_keyboard_input(code, 0, KEYEVENTF_KEYUP) for code in reversed(codes)]
    )
    _send_inputs(events)
    _ = _require_same_frame_after_mutation(permit)


def _keyboard_input(virtual_key: int, scan_code: int, flags: int) -> Input:
    return Input(
        type=INPUT_KEYBOARD,
        value=InputValue(
            keyboard=KeyboardInput(
                virtual_key=virtual_key,
                scan_code=scan_code,
                flags=flags,
                time=0,
                extra_info=0,
            )
        ),
    )


def _send_inputs(events: tuple[Input, ...]) -> None:
    if not events:
        return
    native_events = (Input * len(events))(*events)
    sent = int(USER32.SendInput(len(events), native_events, ctypes.sizeof(Input)))
    if sent != len(events):
        raise ComputerControlError(
            "Windows rejected computer input. An elevated or secure window may require direct user control."
        )


def _move_to_window_point(
    permit: ComputerActionPermit,
    x: int,
    y: int,
) -> ResolvedWindow:
    current = _require_active(permit)
    _validate_point(current, x, y)
    _set_cursor(current.entry.left + x, current.entry.top + y)
    return current


def _set_cursor(x: int, y: int) -> None:
    if not USER32.SetCursorPos(x, y):
        raise ComputerControlError("Windows could not move the pointer.")


def _validate_point(current: ResolvedWindow, x: int, y: int) -> None:
    if x < 0 or y < 0 or x >= current.entry.width or y >= current.entry.height:
        raise ComputerControlError("The requested point is outside the current window.")


def _require_active(permit: ComputerActionPermit) -> ResolvedWindow:
    permit.require_active()
    return require_matching_active_device_window(permit.identity)


def _require_same_frame(
    permit: ComputerActionPermit,
) -> ResolvedWindow:
    permit.require_active()
    return require_same_active_device_window_after_mutation(permit.identity)


def _require_same_frame_after_mutation(
    permit: ComputerActionPermit,
) -> ResolvedWindow:
    permit.require_active()
    return require_same_active_device_window_after_mutation(permit.identity)


def _normalize_key(key: str) -> str:
    normalized = key.strip().upper().replace(" ", "")
    aliases = {"CONTROL": "CTRL", "ESCAPE": "ESC", "WINDOWS": "WIN", "META": "WIN"}
    return aliases.get(normalized, normalized)


def _key_code(key: str) -> int:
    if key in KEY_CODES:
        return KEY_CODES[key]
    if len(key) == 1 and ("A" <= key <= "Z" or "0" <= key <= "9"):
        return ord(key)
    if key.startswith("F") and key[1:].isdigit():
        number = int(key[1:])
        if 1 <= number <= 24:
            return 0x6F + number
    raise ComputerControlError(f"Unsupported keyboard key: {key}")

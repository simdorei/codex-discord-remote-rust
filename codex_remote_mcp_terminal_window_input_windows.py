from __future__ import annotations

# pyright: reportAny=false

import ctypes
import struct
from typing import Final

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_identity_windows import (
    activate_terminal_window,
    require_active_terminal_window,
    require_terminal_window_rect,
)
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from codex_remote_mcp_windows_native import USER32, Input, InputValue, KeyboardInput

_INPUT_KEYBOARD: Final = 1
_KEYEVENTF_KEYUP: Final = 0x0002
_KEYEVENTF_UNICODE: Final = 0x0004
_MODIFIER_CODES: Final = {"CTRL": 0x11, "SHIFT": 0x10, "ALT": 0x12}
_KEY_CODES: Final = {
    "BACKSPACE": 0x08,
    "TAB": 0x09,
    "ENTER": 0x0D,
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
    "INSERT": 0x2D,
    "DELETE": 0x2E,
}


def type_terminal_window_text(
    window: OwnedTerminalWindow,
    text: str,
) -> bool:
    _ = require_terminal_window_rect(window)
    activated = activate_terminal_window(window)
    _send_inputs(_unicode_inputs(text))
    require_active_terminal_window(window)
    return activated


def press_terminal_window_keys(
    window: OwnedTerminalWindow,
    keys: tuple[str, ...],
) -> tuple[bool, tuple[str, ...]]:
    normalized = normalize_terminal_keys(keys)
    _ = require_terminal_window_rect(window)
    activated = activate_terminal_window(window)
    try:
        _send_inputs(_key_inputs(normalized))
    except TerminalExecutionError:
        _best_effort_release(normalized)
        raise
    require_active_terminal_window(window)
    return activated, normalized


def normalize_terminal_keys(keys: tuple[str, ...]) -> tuple[str, ...]:
    normalized = tuple(key.strip().upper().replace(" ", "") for key in keys)
    if any(not key for key in normalized) or len(set(normalized)) != len(normalized):
        raise TerminalExecutionError("terminal key names must be unique and non-empty")
    if any(key in {"WIN", "WINDOWS", "META", "COMMAND"} for key in normalized):
        raise TerminalExecutionError("system-wide keys are not terminal-bound")
    non_modifiers = tuple(key for key in normalized if key not in _MODIFIER_CODES)
    if len(non_modifiers) != 1:
        raise TerminalExecutionError("one non-modifier terminal key is required")
    _ = tuple(_key_code(key) for key in normalized)
    return tuple(
        key for key in ("CTRL", "SHIFT", "ALT") if key in normalized
    ) + non_modifiers


def _unicode_inputs(text: str) -> tuple[Input, ...]:
    raw = text.encode("utf-16-le")
    units = struct.unpack(f"<{len(raw) // 2}H", raw)
    return tuple(
        event
        for unit in units
        for event in (
            _keyboard_input(0, unit, _KEYEVENTF_UNICODE),
            _keyboard_input(0, unit, _KEYEVENTF_UNICODE | _KEYEVENTF_KEYUP),
        )
    )


def _key_inputs(keys: tuple[str, ...]) -> tuple[Input, ...]:
    codes = tuple(_key_code(key) for key in keys)
    return tuple(_keyboard_input(code, 0, 0) for code in codes) + tuple(
        _keyboard_input(code, 0, _KEYEVENTF_KEYUP) for code in reversed(codes)
    )


def _key_code(key: str) -> int:
    if key in _MODIFIER_CODES:
        return _MODIFIER_CODES[key]
    if key in _KEY_CODES:
        return _KEY_CODES[key]
    if len(key) == 1 and ("A" <= key <= "Z" or "0" <= key <= "9"):
        return ord(key)
    if key.startswith("F") and key[1:].isdigit() and 1 <= int(key[1:]) <= 12:
        return 0x6F + int(key[1:])
    raise TerminalExecutionError(f"unsupported terminal key: {key}")


def _keyboard_input(virtual_key: int, scan_code: int, flags: int) -> Input:
    value = InputValue()
    value.keyboard = KeyboardInput(virtual_key, scan_code, flags, 0, 0)
    return Input(type=_INPUT_KEYBOARD, value=value)


def _send_inputs(inputs: tuple[Input, ...]) -> None:
    array = (Input * len(inputs))(*inputs)
    sent = int(USER32.SendInput(len(inputs), array, ctypes.sizeof(Input)))
    if sent != len(inputs):
        raise TerminalExecutionError("Windows did not accept every terminal key event")


def _best_effort_release(keys: tuple[str, ...]) -> None:
    releases = tuple(
        _keyboard_input(_key_code(key), 0, _KEYEVENTF_KEYUP)
        for key in reversed(keys)
    )
    try:
        _send_inputs(releases)
    except TerminalExecutionError:
        return


__all__ = [
    "normalize_terminal_keys",
    "press_terminal_window_keys",
    "type_terminal_window_text",
]

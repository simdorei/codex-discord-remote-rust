from __future__ import annotations

from collections.abc import Callable
from typing import NoReturn

from codex_thread_models import WindowInfo


class DesktopAutomationUnsupportedError(RuntimeError):
    pass


def _unsupported() -> NoReturn:
    raise DesktopAutomationUnsupportedError(
        "Codex Desktop UI automation is only implemented on Windows in this build."
    )


def get_window_text_length(hwnd: int) -> int:
    _ = hwnd
    _unsupported()


def read_window_text(hwnd: int, max_count: int) -> str:
    _ = (hwnd, max_count)
    _unsupported()


def enum_windows(callback: Callable[[int], bool]) -> None:
    _ = callback
    _unsupported()


def get_window_rect_tuple(hwnd: int) -> tuple[int, int, int, int] | None:
    _ = hwnd
    _unsupported()


def is_window_visible(hwnd: int) -> bool:
    _ = hwnd
    _unsupported()


def get_foreground_window() -> int:
    _unsupported()


def show_window(hwnd: int, command: int) -> None:
    _ = (hwnd, command)
    _unsupported()


def set_foreground_window(hwnd: int) -> None:
    _ = hwnd
    _unsupported()


def bring_window_to_top(hwnd: int) -> None:
    _ = hwnd
    _unsupported()


def get_clipboard_text() -> str | None:
    _unsupported()


def set_clipboard_text(text: str) -> None:
    _ = text
    _unsupported()


def send_key_event(vk: int, keyup: bool = False) -> None:
    _ = (vk, keyup)
    _unsupported()


def send_hotkey(*keys: int) -> None:
    _ = keys
    _unsupported()


def click_window(window: WindowInfo, x_ratio: float, y_offset: int) -> tuple[int, int]:
    _ = (window, x_ratio, y_offset)
    _unsupported()

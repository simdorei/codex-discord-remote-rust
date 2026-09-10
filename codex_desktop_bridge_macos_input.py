from __future__ import annotations

import subprocess
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass

from codex_thread_models import WindowInfo


class MacOSAutomationError(RuntimeError):
    pass


class MacOSClipboardError(MacOSAutomationError):
    pass


WindowCallback = Callable[[int], bool]


@dataclass(frozen=True, slots=True)
class MacOSWindowSnapshot:
    handle: int
    pid: int
    process_name: str
    title: str
    left: int
    top: int
    right: int
    bottom: int
    frontmost: bool


_WINDOWS: dict[int, MacOSWindowSnapshot] = {}

VK_CONTROL = 0x11
VK_MENU = 0x12
VK_SHIFT = 0x10
VK_BACK = 0x08
VK_TAB = 0x09
VK_RETURN = 0x0D
VK_ESCAPE = 0x1B

_LETTER_KEYS = {
    0x41: "a",
    0x43: "c",
    0x4A: "j",
    0x4C: "l",
    0x56: "v",
}
_KEY_CODES = {
    VK_BACK: 51,
    VK_TAB: 48,
    VK_RETURN: 36,
    VK_ESCAPE: 53,
}
_MODIFIERS = {
    VK_CONTROL: "command down",
    VK_MENU: "option down",
    VK_SHIFT: "shift down",
}


def _run(
    args: Sequence[str],
    *,
    input_text: str | None = None,
    timeout: float = 10.0,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(args),
        input=input_text,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )


def _osascript(lines: Sequence[str], *, timeout: float = 10.0) -> str:
    args: list[str] = ["osascript"]
    for line in lines:
        args.extend(["-e", line])
    completed = _run(args, timeout=timeout)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or f"exit={completed.returncode}").strip()
        raise MacOSAutomationError(detail or "osascript failed")
    return completed.stdout


def _quote(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def _window_snapshot_script() -> list[str]:
    return [
        "set rows to {}",
        'set tabText to ASCII character 9',
        'set lfText to ASCII character 10',
        'tell application "System Events"',
        "repeat with proc in application processes",
        "try",
        "if visible of proc then",
        "set procName to name of proc as text",
        "set procPid to unix id of proc as text",
        "set procFront to frontmost of proc as text",
        "repeat with win in windows of proc",
        "try",
        "set winName to name of win as text",
        "set winPos to position of win",
        "set winSize to size of win",
        "set rowText to procPid & tabText & procName & tabText & procFront & tabText & "
        + "(item 1 of winPos) & tabText & (item 2 of winPos) & tabText & "
        + "(item 1 of winSize) & tabText & (item 2 of winSize) & tabText & winName",
        "set end of rows to rowText",
        "end try",
        "end repeat",
        "end if",
        "end try",
        "end repeat",
        "end tell",
        "set AppleScript's text item delimiters to lfText",
        "return rows as text",
    ]


def _parse_bool(value: str) -> bool:
    return value.strip().lower() in {"true", "yes", "1"}


def _refresh_windows() -> list[MacOSWindowSnapshot]:
    output = _osascript(_window_snapshot_script())
    snapshots: list[MacOSWindowSnapshot] = []
    for index, raw_line in enumerate(line for line in output.splitlines() if line.strip()):
        parts = raw_line.split("\t", 7)
        if len(parts) != 8:
            continue
        pid_text, process_name, frontmost, left, top, width, height, title = parts
        try:
            pid = int(pid_text)
            left_i = int(float(left))
            top_i = int(float(top))
            width_i = int(float(width))
            height_i = int(float(height))
        except ValueError:
            continue
        handle = (pid * 1000) + index + 1
        snapshots.append(
            MacOSWindowSnapshot(
                handle=handle,
                pid=pid,
                process_name=process_name,
                title=title,
                left=left_i,
                top=top_i,
                right=left_i + width_i,
                bottom=top_i + height_i,
                frontmost=_parse_bool(frontmost),
            )
        )
    _WINDOWS.clear()
    _WINDOWS.update({window.handle: window for window in snapshots})
    return snapshots


def _get_window(hwnd: int) -> MacOSWindowSnapshot:
    window = _WINDOWS.get(hwnd)
    if window is None:
        _ = _refresh_windows()
        window = _WINDOWS.get(hwnd)
    if window is None:
        raise MacOSAutomationError(f"macOS window handle not found: {hwnd}")
    return window


def get_window_process_id(hwnd: int) -> int | None:
    try:
        return _get_window(hwnd).pid
    except MacOSAutomationError:
        return None


def get_window_text_length(hwnd: int) -> int:
    return len(_get_window(hwnd).title)


def read_window_text(hwnd: int, max_count: int) -> str:
    title = _get_window(hwnd).title
    return title[: max(max_count - 1, 0)]


def enum_windows(callback: WindowCallback) -> None:
    for window in _refresh_windows():
        if not callback(window.handle):
            break


def get_window_rect_tuple(hwnd: int) -> tuple[int, int, int, int] | None:
    window = _get_window(hwnd)
    return (window.left, window.top, window.right, window.bottom)


def is_window_visible(hwnd: int) -> bool:
    _ = _get_window(hwnd)
    return True


def get_foreground_window() -> int:
    for window in _refresh_windows():
        if window.frontmost:
            return window.handle
    return 0


def _raise_window(hwnd: int) -> None:
    window = _get_window(hwnd)
    _ = _osascript(
        [
            'tell application "System Events"',
            f"tell process {_quote(window.process_name)}",
            "set frontmost to true",
            f"set targetTitle to {_quote(window.title)}",
            "repeat with win in windows",
            "if (name of win as text) is targetTitle then",
            'try to perform action "AXRaise" of win',
            "exit repeat",
            "end if",
            "end repeat",
            "end tell",
            "end tell",
        ]
    )


def show_window(hwnd: int, command: int) -> None:
    _ = command
    _raise_window(hwnd)


def set_foreground_window(hwnd: int) -> None:
    _raise_window(hwnd)


def bring_window_to_top(hwnd: int) -> None:
    _raise_window(hwnd)


def get_clipboard_text() -> str | None:
    completed = _run(["pbpaste"], timeout=5.0)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or f"exit={completed.returncode}").strip()
        raise MacOSClipboardError(detail or "pbpaste failed")
    return completed.stdout


def set_clipboard_text(text: str) -> None:
    completed = _run(["pbcopy"], input_text=text, timeout=5.0)
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or f"exit={completed.returncode}").strip()
        raise MacOSClipboardError(detail or "pbcopy failed")


def send_key_event(vk: int, keyup: bool = False) -> None:
    if keyup or vk in _MODIFIERS:
        return
    key_code = _KEY_CODES.get(vk)
    if key_code is None:
        letter = _LETTER_KEYS.get(vk)
        if letter is None:
            raise MacOSAutomationError(f"Unsupported macOS key event: {vk}")
        _ = _osascript(['tell application "System Events"', f"keystroke {_quote(letter)}", "end tell"])
        return
    _ = _osascript(['tell application "System Events"', f"key code {key_code}", "end tell"])


def send_hotkey(*keys: int) -> None:
    modifiers = [_MODIFIERS[key] for key in keys if key in _MODIFIERS]
    normal_keys = [key for key in keys if key not in _MODIFIERS]
    if not normal_keys:
        return
    key = normal_keys[-1]
    using = " using {" + ", ".join(modifiers) + "}" if modifiers else ""
    if key in _LETTER_KEYS:
        _ = _osascript(['tell application "System Events"', f"keystroke {_quote(_LETTER_KEYS[key])}{using}", "end tell"])
        return
    if key in _KEY_CODES:
        _ = _osascript(['tell application "System Events"', f"key code {_KEY_CODES[key]}{using}", "end tell"])
        return
    raise MacOSAutomationError(f"Unsupported macOS hotkey: {keys}")


def click_window(window: WindowInfo, x_ratio: float, y_offset: int) -> tuple[int, int]:
    x = int(window.left + ((window.right - window.left) * x_ratio))
    y = int(window.top + y_offset)
    _ = _osascript(['tell application "System Events"', f"click at {{{x}, {y}}}", "end tell"])
    return (x, y)


def _find_codex_window() -> MacOSWindowSnapshot:
    for window in _refresh_windows():
        if window.process_name == "Codex" or window.title == "Codex" or window.title.startswith("Codex - "):
            return window
    raise MacOSAutomationError("Visible Codex Desktop window not found.")


class _MacOSUIBackend:
    def quote(self, value: str) -> str:
        return _quote(value)

    def osascript(self, lines: Sequence[str], *, timeout: float = 10.0) -> str:
        return _osascript(lines, timeout=timeout)

    def find_codex_window(self) -> MacOSWindowSnapshot:
        return _find_codex_window()

    def raise_window(self, hwnd: int) -> None:
        _raise_window(hwnd)

    def refresh_windows(self) -> list[MacOSWindowSnapshot]:
        return _refresh_windows()


MACOS_UI_BACKEND = _MacOSUIBackend()


def run_composer_focus_process(args: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
    import codex_desktop_bridge_macos_ui as macos_ui

    return macos_ui.run_composer_focus_process(args, backend=MACOS_UI_BACKEND)


def run_header_verification_process(
    args: list[str], *, env: Mapping[str, str] | None = None, **_kwargs: object
) -> subprocess.CompletedProcess[str]:
    import codex_desktop_bridge_macos_ui as macos_ui

    return macos_ui.run_header_verification_process(args, backend=MACOS_UI_BACKEND, env=env)


def run_sidebar_activation_process(
    args: list[str], *, env: Mapping[str, str] | None = None, **_kwargs: object
) -> subprocess.CompletedProcess[str]:
    import codex_desktop_bridge_macos_ui as macos_ui

    return macos_ui.run_sidebar_activation_process(args, backend=MACOS_UI_BACKEND, env=env)


def run_permission_approval_process(
    args: list[str], *, env: Mapping[str, str] | None = None, **_kwargs: object
) -> subprocess.CompletedProcess[str]:
    import codex_desktop_bridge_macos_ui as macos_ui

    return macos_ui.run_permission_approval_process(args, backend=MACOS_UI_BACKEND, env=env)

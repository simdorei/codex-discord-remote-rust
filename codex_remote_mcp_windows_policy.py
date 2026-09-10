from __future__ import annotations

import os
from pathlib import PureWindowsPath
from typing import Final

from codex_remote_mcp_computer_errors import ComputerControlError

SECURITY_TITLE_MARKERS: Final = (
    "1password",
    "additional verification",
    "authentication",
    "authenticator",
    "bitwarden",
    "chatgpt",
    "chrome remote desktop",
    "codex",
    "credential manager",
    "developer tools",
    "devtools",
    "enter your password",
    "enter code",
    "google accounts",
    "lastpass",
    "one-time code",
    "one-time password",
    "passcode",
    "password",
    "privacy & security",
    "sign in",
    "sign-in",
    "two-step verification",
    "2-step verification",
    "verification",
    "verification code",
    "verify it's you",
    "confirm your identity",
    "account recovery",
    "recovery code",
    "security check",
    "security code",
    "access code",
    "windows security",
    "\ub85c\uadf8\uc778",
    "\ubcf8\uc778\uc778\uc99d",
    "\ube44\ubc00\ubc88\ud638",
    "\uc554\ud638 \uc785\ub825",
    "\uc778\uc99d\ubc88\ud638",
    "\ubcf4\uc548 \ucf54\ub4dc",
    "\ubcf8\uc778 \ud655\uc778",
    "\uacc4\uc815 \ubcf5\uad6c",
    "\ucd94\uac00\uc778\uc99d",
    "\uac1c\uc778 \uc815\ubcf4 \ubc0f \ubcf4\uc548",
)
PROTECTED_TITLES: Final = frozenset(
    {
        "open",
        "page setup",
        "print",
        "run",
        "save as",
        "\uc5f4\uae30",
        "\uc2e4\ud589",
        "\uc778\uc1c4",
        "\ub2e4\ub978 \uc774\ub984\uc73c\ub85c \uc800\uc7a5",
    }
)
KEY_CODES: Final = {
    "BACKSPACE": 0x08,
    "TAB": 0x09,
    "ENTER": 0x0D,
    "SHIFT": 0x10,
    "CTRL": 0x11,
    "ALT": 0x12,
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
    "DELETE": 0x2E,
}
BLOCKED_KEY_NAMES: Final = frozenset({"WIN", "WINDOWS", "META", "COMMAND", "F12"})
ALLOWED_CHORDS: Final = frozenset(
    {
        frozenset({key})
        for key in (
            "BACKSPACE",
            "DELETE",
            "DOWN",
            "END",
            "ENTER",
            "ESC",
            "HOME",
            "LEFT",
            "PAGEDOWN",
            "PAGEUP",
            "RIGHT",
            "SPACE",
            "TAB",
            "UP",
        )
    }
    | {frozenset({"CTRL", key}) for key in ("A", "C", "X", "Z")}
)
SUPPORTED_INTERACTIONS: Final = frozenset(
    {"click", "drag", "scroll", "type_text", "press_keys"}
)


def require_allowed_window(process_path: str, title: str) -> tuple[str, str]:
    normalized_title = " ".join(title.casefold().split())
    if normalized_title in PROTECTED_TITLES or any(
        marker in normalized_title for marker in SECURITY_TITLE_MARKERS
    ):
        raise ComputerControlError(
            "This security, sign-in, or protected window requires direct user control."
        )
    app = _allowed_application(process_path)
    if app == "chrome":
        return "Google Chrome", "chrome.exe"
    return "Notepad", "notepad.exe"


def parse_key_chord(keys: tuple[str, ...]) -> tuple[int, ...]:
    normalized = tuple(_normalize_key(key) for key in keys)
    if len(normalized) != len(set(normalized)):
        raise ComputerControlError("Duplicate keyboard keys are not available.")
    if any(key in BLOCKED_KEY_NAMES for key in normalized):
        raise ComputerControlError("This protected keyboard shortcut is not available.")
    normalized_set = frozenset(normalized)
    if normalized_set not in ALLOWED_CHORDS:
        raise ComputerControlError("This protected keyboard shortcut is not available.")
    return tuple(_key_code(key) for key in normalized)


def require_allowed_interaction(process_path: str, interaction: str) -> None:
    if interaction not in SUPPORTED_INTERACTIONS:
        raise ComputerControlError("This computer interaction is not available.")
    app = _allowed_application(process_path)
    if app == "chrome":
        raise ComputerControlError(
            "Chrome interactions require direct user control so authentication, "
            + "CAPTCHA, consent, secret fields, and page content stay private."
        )


def _allowed_application(process_path: str) -> str:
    path = _normalized_path(process_path)
    if path.name == "chrome.exe" and _is_standard_chrome_path(path):
        return "chrome"
    if path.name == "notepad.exe" and _is_standard_notepad_path(path):
        return "notepad"
    raise ComputerControlError(
        "This protected or unapproved application cannot be controlled."
    )


def _normalized_path(value: str) -> PureWindowsPath:
    if not value or not PureWindowsPath(value).is_absolute():
        raise ComputerControlError("Windows could not verify the application path.")
    return PureWindowsPath(value.casefold())


def _is_standard_chrome_path(path: PureWindowsPath) -> bool:
    suffix = PureWindowsPath("google/chrome/application/chrome.exe")
    for variable in ("PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"):
        base = _normalized_base(variable)
        if base is not None and path == base / suffix:
            return True
    return False


def _is_standard_notepad_path(path: PureWindowsPath) -> bool:
    windows = _normalized_base("SYSTEMROOT")
    if windows is not None and path == windows / "system32/notepad.exe":
        return True
    program_files = _normalized_base("PROGRAMFILES")
    if program_files is None:
        return False
    windows_apps = program_files / "windowsapps"
    try:
        relative = path.relative_to(windows_apps)
    except ValueError:
        return False
    parts = relative.parts
    return (
        len(parts) >= 2
        and parts[0].startswith("microsoft.windowsnotepad_")
        and parts[-1] == "notepad.exe"
    )


def _normalized_base(variable: str) -> PureWindowsPath | None:
    value = os.environ.get(variable)
    if not value:
        return None
    return PureWindowsPath(value.casefold())


def _normalize_key(key: str) -> str:
    normalized = key.strip().upper().replace(" ", "")
    if not normalized:
        raise ComputerControlError("Keyboard key names cannot be empty.")
    return normalized


def _key_code(key: str) -> int:
    if key in KEY_CODES:
        return KEY_CODES[key]
    if len(key) == 1 and ("A" <= key <= "Z" or "0" <= key <= "9"):
        return ord(key)
    if key.startswith("F") and key[1:].isdigit():
        number = int(key[1:])
        if 1 <= number <= 11:
            return 0x6F + number
    raise ComputerControlError(f"Unsupported keyboard key: {key}")

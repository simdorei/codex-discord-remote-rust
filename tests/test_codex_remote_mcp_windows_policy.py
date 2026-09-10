from __future__ import annotations

import pytest

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_policy import (
    parse_key_chord,
    require_allowed_interaction,
    require_allowed_window,
)


def test_policy_allows_only_standard_chrome_and_notepad_paths(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("PROGRAMFILES", r"C:\Program Files")
    monkeypatch.setenv("SYSTEMROOT", r"C:\Windows")

    assert require_allowed_window(
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        "Example",
    ) == ("Google Chrome", "chrome.exe")
    assert require_allowed_window(
        r"C:\Windows\System32\notepad.exe",
        "notes.txt - Notepad",
    ) == ("Notepad", "notepad.exe")
    for process_path in (
        r"C:\Temp\chrome.exe",
        r"C:\Windows\explorer.exe",
        r"C:\Windows\System32\control.exe",
        r"C:\Windows\System32\cmd.exe",
    ):
        with pytest.raises(ComputerControlError, match="protected or unapproved"):
            _ = require_allowed_window(process_path, "Safe-looking title")


@pytest.mark.parametrize(
    ("process_path", "title"),
    (
        (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "ChatGPT"),
        (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Codex"),
        (r"C:\Windows\explorer.exe", "Run"),
        (r"C:\Windows\explorer.exe", "\uc2e4\ud589"),
        (r"C:\Windows\System32\SystemSettings.exe", "Privacy & security"),
        (r"C:\Windows\System32\SystemSettings.exe", "\ub85c\uadf8\uc778"),
        (r"C:\Windows\System32\notepad.exe", "Save As"),
        (r"C:\Windows\System32\notepad.exe", "\uc5f4\uae30"),
        (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Google Accounts"),
        (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Enter code"),
        (
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "Two-step verification",
        ),
        (
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "Verify it's you",
        ),
        (
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "Confirm your identity",
        ),
        (
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "\ubcf4\uc548 \ucf54\ub4dc \uc785\ub825",
        ),
    ),
)
def test_policy_blocks_protected_titles(process_path: str, title: str) -> None:
    with pytest.raises(ComputerControlError, match="protected window"):
        _ = require_allowed_window(process_path, title)


@pytest.mark.parametrize(
    "keys",
    (
        ("WIN", "R"),
        ("ALT", "F4", "SHIFT"),
        ("CTRL", "SHIFT", "ESC", "ALT"),
        ("CTRL", "SHIFT", "J"),
        ("F12",),
        ("CTRL", "O"),
        ("CTRL", "V"),
        ("CTRL", "S", "SHIFT"),
        ("ALT", "TAB"),
        ("CTRL", "SHIFT", "A"),
    ),
)
def test_policy_blocks_protected_shortcuts_and_supersets(
    keys: tuple[str, ...],
) -> None:
    with pytest.raises(ComputerControlError, match="protected keyboard shortcut"):
        _ = parse_key_chord(keys)


def test_policy_allows_plain_editing_shortcut() -> None:
    assert parse_key_chord(("CTRL", "A")) == (0x11, 0x41)


@pytest.mark.parametrize(
    "interaction",
    ("click", "drag", "scroll", "type_text", "press_keys"),
)
def test_policy_blocks_all_chrome_interactions(
    interaction: str,
) -> None:
    with pytest.raises(ComputerControlError, match="direct user control"):
        require_allowed_interaction(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            interaction,
        )


def test_policy_allows_notepad_editing() -> None:
    require_allowed_interaction(
        r"C:\Windows\System32\notepad.exe",
        "type_text",
    )

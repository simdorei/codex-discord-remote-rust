from __future__ import annotations

import pytest

import codex_remote_mcp_windows_platform as windows_platform
import codex_remote_mcp_windows_screenshot as windows_screenshot
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry
from tests.remote_mcp_computer_fakes import launched_application


def _resolved(
    *,
    width: int = 800,
    process_path: str = r"c:\windows\system32\notepad.exe",
    process_name: str = "notepad.exe",
) -> ResolvedWindow:
    entry = ComputerWindowEntry(
        window_id=42,
        title="Notepad",
        process_name=process_name,
        left=10,
        top=20,
        width=width,
        height=600,
        active=True,
    )
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=entry.window_id,
            process_id=123,
            process_path=process_path,
            title_digest="a" * 64,
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
            surface_window_id=84,
            surface_digest="c" * 64,
            surface_left=20,
            surface_top=50,
            surface_width=760,
            surface_height=520,
        ),
    )


def test_screenshot_rejects_oversized_window_before_native_allocation(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured = False

    def capture(*_: int) -> bytes:
        nonlocal captured
        captured = True
        return b"unused"

    oversized = _resolved(width=5_000)
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(oversized),
    )
    monkeypatch.setattr(windows_platform, "resolve_allowed_window", lambda _: oversized)
    monkeypatch.setattr(
        windows_screenshot,
        "require_matching_active_window",
        lambda _: oversized,
    )
    monkeypatch.setattr(windows_screenshot, "capture_window", capture)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="too large to capture"):
        platform.screenshot(42)

    assert captured is False


def test_chrome_screenshot_is_rejected_before_pixel_capture(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    chrome = _resolved(
        process_path=r"c:\program files\google\chrome\application\chrome.exe",
        process_name="chrome.exe",
    )
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(chrome),
    )
    monkeypatch.setattr(windows_platform, "resolve_allowed_window", lambda _: chrome)
    monkeypatch.setattr(
        windows_screenshot,
        "capture_window",
        lambda *_: pytest.fail("Chrome pixels must not be captured"),
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("chrome")

    with pytest.raises(
        ComputerControlError, match="Chrome screenshots are unavailable"
    ):
        _ = platform.screenshot(chrome.entry.window_id)

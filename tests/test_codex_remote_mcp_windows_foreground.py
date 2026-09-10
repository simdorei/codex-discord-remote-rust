from __future__ import annotations

import pytest

import codex_remote_mcp_windows_input as windows_input
import codex_remote_mcp_windows_platform as windows_platform
import codex_remote_mcp_windows_screenshot as windows_screenshot
from codex_remote_mcp_computer_contracts import (
    ComputerActionPermit,
    ComputerCapture,
    ComputerWindowIdentity,
)
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry
from tests.remote_mcp_computer_fakes import FakeOwnedProcess, launched_application


class _Permit:
    def __init__(self, identity: ComputerWindowIdentity) -> None:
        self.identity = identity

    def require_active(self) -> None:
        return None


def _resolved() -> ResolvedWindow:
    entry = ComputerWindowEntry(
        window_id=42,
        title="Notepad",
        process_name="notepad.exe",
        left=10,
        top=20,
        width=800,
        height=600,
        active=True,
    )
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=entry.window_id,
            process_id=123,
            process_path=r"c:\windows\system32\notepad.exe",
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


def test_screenshot_checks_foreground_before_and_after_pixel_capture(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    active_checks = 0

    def require_active(_: ComputerWindowIdentity) -> ResolvedWindow:
        nonlocal active_checks
        active_checks += 1
        if active_checks == 2:
            raise ComputerControlError("The active window changed.")
        return resolved

    monkeypatch.setattr(
        windows_screenshot,
        "require_matching_active_window",
        require_active,
    )
    monkeypatch.setattr(windows_screenshot, "capture_window", lambda *_: b"png")

    with pytest.raises(ComputerControlError, match="active window changed"):
        _ = windows_screenshot.capture_resolved_window(resolved)

    assert active_checks == 2


def test_notepad_action_rejects_a_foreground_change_before_input(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()

    def reject_inactive(_: ComputerWindowIdentity) -> ResolvedWindow:
        raise ComputerControlError("The active window changed.")

    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        reject_inactive,
    )
    monkeypatch.setattr(
        windows_input,
        "_send_message",
        lambda *_: pytest.fail("input must not be sent"),
    )

    with pytest.raises(ComputerControlError, match="active window changed"):
        windows_input.click_window(_Permit(resolved.identity), 30, 40, "left", 1)


def test_recycled_pid_and_window_are_rejected_when_owned_process_exited(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    process = FakeOwnedProcess(resolved.identity.process_id)
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved, process=process),
    )
    monkeypatch.setattr(
        windows_platform,
        "resolve_allowed_window",
        lambda _: pytest.fail("a recycled window must not be trusted"),
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")
    process.exit_code = 0

    with pytest.raises(ComputerControlError, match="process is no longer running"):
        _ = platform.screenshot(resolved.entry.window_id)


def test_window_is_rejected_when_retained_process_handle_has_another_pid(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    process = FakeOwnedProcess(999)
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved, process=process),
    )
    monkeypatch.setattr(
        windows_platform,
        "resolve_allowed_window",
        lambda _: pytest.fail("a mismatched process must not be trusted"),
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="process identity changed"):
        _ = platform.screenshot(resolved.entry.window_id)


def test_process_exit_during_input_is_rejected_before_success(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    process = FakeOwnedProcess(resolved.identity.process_id)
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved, process=process),
    )

    def exit_during_input(
        permit: ComputerActionPermit,
        x: int,
        y: int,
        button: str,
        click_count: int,
    ) -> None:
        _ = x, y, button, click_count
        permit.require_active()
        process.exit_code = 0
        permit.require_active()

    monkeypatch.setattr(windows_platform, "click_window", exit_during_input)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="process is no longer running"):
        platform.click(_Permit(resolved.identity), 30, 40, "left", 1)


def test_process_exit_during_screenshot_prevents_returning_pixels(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    process = FakeOwnedProcess(resolved.identity.process_id)
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved, process=process),
    )
    monkeypatch.setattr(windows_platform, "resolve_allowed_window", lambda _: resolved)

    def exit_during_capture(_: ResolvedWindow) -> ComputerCapture:
        process.exit_code = 0
        return ComputerCapture(
            window=resolved.entry, identity=resolved.identity, png=b"png"
        )

    monkeypatch.setattr(
        windows_platform,
        "capture_resolved_window",
        exit_during_capture,
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="process is no longer running"):
        _ = platform.screenshot(resolved.entry.window_id)

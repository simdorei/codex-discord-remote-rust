from __future__ import annotations

import ctypes

import pytest

import codex_remote_mcp_windows_platform as windows_platform
import codex_remote_mcp_windows_text as windows_text
import codex_remote_mcp_windows_windows as windows_windows
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_platform_lifecycle import OwnedLaunch
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry
from tests.remote_mcp_computer_fakes import launched_application


class _Permit:
    def __init__(self, identity: ComputerWindowIdentity) -> None:
        self.identity = identity

    def require_active(self) -> None:
        return None


def _resolved(
    *,
    width: int = 800,
    process_id: int = 123,
    process_path: str = r"c:\windows\system32\notepad.exe",
    title_digest: str = "a" * 64,
    surface_digest: str = "c" * 64,
) -> ResolvedWindow:
    entry = ComputerWindowEntry(
        window_id=42,
        title="Notepad",
        process_name="notepad.exe",
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
            process_id=process_id,
            process_path=process_path,
            title_digest=title_digest,
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
            surface_window_id=84,
            surface_digest=surface_digest,
            surface_left=20,
            surface_top=50,
            surface_width=760,
            surface_height=520,
        ),
    )


def test_window_identity_rejects_handle_reuse_or_process_change(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    expected = _resolved(process_id=123).identity
    monkeypatch.setattr(
        windows_windows,
        "resolve_allowed_window",
        lambda _: _resolved(process_id=999),
    )

    with pytest.raises(ComputerControlError, match="window changed"):
        windows_windows.require_matching_window(expected)


def test_chrome_action_rejects_page_title_change_after_screenshot(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    chrome_path = r"c:\program files\google\chrome\application\chrome.exe"
    expected = _resolved(process_path=chrome_path, title_digest="a" * 64).identity
    monkeypatch.setattr(
        windows_windows,
        "resolve_allowed_window",
        lambda _: _resolved(process_path=chrome_path, title_digest="b" * 64),
    )

    with pytest.raises(ComputerControlError, match="window changed"):
        windows_windows.require_matching_window(expected)


def test_notepad_action_rejects_surface_content_change(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    expected = _resolved(surface_digest="a" * 64).identity
    monkeypatch.setattr(
        windows_windows,
        "resolve_allowed_window",
        lambda _: _resolved(surface_digest="b" * 64),
    )

    with pytest.raises(ComputerControlError, match="window changed"):
        windows_windows.require_matching_window(expected)


def test_notepad_control_text_uses_a_timeout_bound_cross_process_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    messages: list[int] = []

    def send_message(
        _window_id: int,
        message: int,
        _wparam: int,
        lparam: int,
        _flags: int,
        _timeout: int,
        result_pointer: int,
    ) -> int:
        messages.append(message)
        result = ctypes.cast(
            result_pointer,
            ctypes.POINTER(ctypes.c_size_t),
        )
        if message == windows_text.WM_GETTEXTLENGTH:
            result.contents.value = 7
        else:
            value = ctypes.create_unicode_buffer("changed")
            _ = ctypes.memmove(lparam, value, ctypes.sizeof(value))
            result.contents.value = 7
        return 1

    monkeypatch.setattr(
        windows_text.USER32,
        "SendMessageTimeoutW",
        send_message,
    )

    assert windows_text.read_control_text(84, limit=100) == "changed"
    assert messages == [windows_text.WM_GETTEXTLENGTH, windows_text.WM_GETTEXT]


def test_notepad_action_rejects_document_change_after_screenshot(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    expected = _resolved(title_digest="a" * 64).identity
    monkeypatch.setattr(
        windows_windows,
        "resolve_allowed_window",
        lambda _: _resolved(title_digest="b" * 64),
    )

    with pytest.raises(ComputerControlError, match="window changed"):
        windows_windows.require_matching_window(expected)


def test_close_rejects_window_that_is_no_longer_active(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    permit = _Permit(_resolved().identity)

    def reject_inactive(_: ComputerWindowIdentity) -> None:
        raise ComputerControlError("The active window changed.")

    monkeypatch.setattr(
        windows_platform,
        "require_matching_active_window",
        reject_inactive,
    )
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(_resolved()),
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="active window changed"):
        platform.close(permit)


def test_unlaunched_window_is_never_read_or_controlled() -> None:
    platform = windows_platform.WindowsComputerPlatform()

    with pytest.raises(ComputerControlError, match="launched by this ChatGPT session"):
        platform.screenshot(42)


def test_successful_close_removes_window_from_session_listing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved),
    )
    monkeypatch.setattr(windows_platform, "resolve_allowed_window", lambda _: resolved)
    monkeypatch.setattr(
        windows_platform,
        "require_matching_active_window",
        lambda _: resolved,
    )
    monkeypatch.setattr(windows_platform, "close_owned_window", lambda *_: None)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    platform.close(_Permit(resolved.identity))

    assert platform.list_windows() == ()


def test_failed_close_verification_keeps_window_owned_for_retry(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    attempts = 0
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved),
    )
    monkeypatch.setattr(
        windows_platform,
        "require_matching_active_window",
        lambda _: resolved,
    )

    def verify_close(
        _: ComputerWindowIdentity,
        __: OwnedLaunch,
    ) -> None:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            raise ComputerControlError("The launched process identity changed.")

    monkeypatch.setattr(windows_platform, "close_owned_window", verify_close)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="process identity changed"):
        platform.close(_Permit(resolved.identity))
    platform.close(_Permit(resolved.identity))

    assert attempts == 2


def test_platform_stop_terminates_each_owned_process_once(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    stopped: list[tuple[int, int, str]] = []
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved),
    )
    monkeypatch.setattr(
        windows_platform,
        "stop_owned_process",
        lambda window_id, process, process_path: stopped.append(
            (window_id, process.pid, process_path)
        ),
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    platform.stop()
    platform.stop()

    assert stopped == [(42, 123, resolved.identity.process_path)]
    assert platform.list_windows() == ()

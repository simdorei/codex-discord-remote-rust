from __future__ import annotations

from dataclasses import replace

import pytest

import codex_remote_mcp_windows_platform as windows_platform
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_launch_types import (
    ApplicationLaunchCleanupError,
    FailedLaunchCleanup,
    LaunchedApplication,
)
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry
from tests.remote_mcp_computer_fakes import (
    FakeOwnedProcess,
    launched_application,
)


class _Permit:
    def __init__(self, identity: ComputerWindowIdentity) -> None:
        self.identity = identity

    def require_active(self) -> None:
        return None


class _RevokedPermit(_Permit):
    def require_active(self) -> None:
        raise ComputerControlError("The screenshot observation is no longer active.")


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


def test_user_closed_window_is_omitted_but_retained_for_process_cleanup(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    stopped: list[tuple[int, int, str]] = []

    def launch_app(_: str) -> LaunchedApplication:
        return launched_application(resolved)

    monkeypatch.setattr(windows_platform, "launch_allowed_app", launch_app)

    def missing_window(_: int) -> ResolvedWindow:
        raise ComputerControlError("The selected window is no longer visible.")

    def record_stop(
        window_id: int,
        process: FakeOwnedProcess,
        process_path: str,
    ) -> None:
        stopped.append((window_id, process.pid, process_path))

    monkeypatch.setattr(windows_platform, "resolve_allowed_window", missing_window)
    monkeypatch.setattr(windows_platform, "stop_owned_process", record_stop)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    assert platform.list_windows() == ()
    platform.stop()

    assert stopped == [(42, 123, resolved.identity.process_path)]


def test_platform_stop_retains_failed_cleanup_for_a_retry(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    attempts: list[int] = []

    def launch_app(_: str) -> LaunchedApplication:
        return launched_application(resolved)

    monkeypatch.setattr(windows_platform, "launch_allowed_app", launch_app)

    def stop_process(
        _window_id: int,
        process: FakeOwnedProcess,
        _process_path: str,
    ) -> None:
        attempts.append(process.pid)
        if len(attempts) == 1:
            raise ComputerControlError("temporary stop failure")

    monkeypatch.setattr(windows_platform, "stop_owned_process", stop_process)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="temporary stop failure"):
        platform.stop()
    platform.stop()

    assert attempts == [123, 123]


def test_platform_stop_attempts_profile_cleanup_after_process_stop_failure(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    cleanup_attempts: list[str | None] = []
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(
            resolved,
            temporary_profile="temporary-profile",
        ),
    )
    monkeypatch.setattr(
        windows_platform,
        "stop_owned_process",
        lambda *_: (_ for _ in ()).throw(ComputerControlError("stop failed")),
    )
    monkeypatch.setattr(
        windows_platform,
        "remove_temporary_profile",
        cleanup_attempts.append,
    )
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("chrome")

    with pytest.raises(ComputerControlError, match="stop failed"):
        platform.stop()

    assert cleanup_attempts == ["temporary-profile"]


def test_platform_retains_a_failed_launch_cleanup_until_stop_succeeds(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    cleanup = FailedLaunchCleanup("chrome", FakeOwnedProcess(), "temporary-profile")
    attempts: list[FailedLaunchCleanup] = []

    def fail_launch(_: str) -> LaunchedApplication:
        raise ApplicationLaunchCleanupError(cleanup)

    def retry(candidate: FailedLaunchCleanup) -> None:
        attempts.append(candidate)
        if len(attempts) == 1:
            raise ApplicationLaunchCleanupError(candidate)

    monkeypatch.setattr(windows_platform, "launch_allowed_app", fail_launch)
    monkeypatch.setattr(windows_platform, "retry_failed_launch_cleanup", retry)
    platform = windows_platform.WindowsComputerPlatform()

    with pytest.raises(ApplicationLaunchCleanupError):
        platform.launch("chrome")
    with pytest.raises(ApplicationLaunchCleanupError):
        platform.stop()
    platform.stop()

    assert attempts == [cleanup, cleanup]


def test_platform_stop_retries_temporary_profile_cleanup(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    process = FakeOwnedProcess()
    cleanup_attempts: list[str | None] = []
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(
            resolved,
            process=process,
            temporary_profile="temporary-profile",
        ),
    )
    monkeypatch.setattr(
        windows_platform,
        "stop_owned_process",
        lambda *_: process.terminate(),
    )

    def cleanup(directory: str | None) -> None:
        cleanup_attempts.append(directory)
        if len(cleanup_attempts) == 1:
            raise ComputerControlError("temporary profile is still locked")

    monkeypatch.setattr(windows_platform, "remove_temporary_profile", cleanup)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("chrome")

    with pytest.raises(ComputerControlError, match="still locked"):
        platform.stop()
    platform.stop()

    assert cleanup_attempts == ["temporary-profile", "temporary-profile"]


def test_platform_stop_retains_process_when_inspection_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    attempts: list[int] = []

    def stop_process(
        _window_id: int,
        process: FakeOwnedProcess,
        _process_path: str,
    ) -> None:
        attempts.append(process.pid)
        if len(attempts) == 1:
            raise ComputerControlError("Windows could not inspect the process")
        process.terminate()

    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(resolved),
    )
    monkeypatch.setattr(windows_platform, "stop_owned_process", stop_process)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="could not inspect"):
        platform.stop()
    platform.stop()

    assert attempts == [123, 123]


def test_clipboard_write_rejects_a_chrome_observation(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    base = _resolved()
    chrome = replace(
        base,
        entry=base.entry.model_copy(
            update={"title": "Chrome", "process_name": "chrome.exe"}
        ),
        identity=replace(
            base.identity,
            process_path=(r"c:\program files\google\chrome\application\chrome.exe"),
            surface_window_id=None,
            surface_width=0,
            surface_height=0,
        ),
    )
    monkeypatch.setattr(
        windows_platform,
        "launch_allowed_app",
        lambda _: launched_application(chrome),
    )
    monkeypatch.setattr(
        windows_platform,
        "require_matching_active_window",
        lambda _: chrome,
    )
    clipboard_writes: list[str] = []
    monkeypatch.setattr(windows_platform, "set_clipboard_text", clipboard_writes.append)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("chrome")

    with pytest.raises(ComputerControlError, match="only in Notepad"):
        platform.set_clipboard(_Permit(chrome.identity), "remote text")

    assert clipboard_writes == []


def test_clipboard_write_rejects_a_revoked_notepad_observation(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
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
    clipboard_writes: list[str] = []
    monkeypatch.setattr(windows_platform, "set_clipboard_text", clipboard_writes.append)
    platform = windows_platform.WindowsComputerPlatform()
    platform.launch("notepad")

    with pytest.raises(ComputerControlError, match="no longer active"):
        platform.set_clipboard(_RevokedPermit(resolved.identity), "remote text")

    assert clipboard_writes == []

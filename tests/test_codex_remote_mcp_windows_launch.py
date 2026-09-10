from __future__ import annotations

import subprocess
import time
from typing import final

import pytest

import codex_remote_mcp_windows_launch as windows_launch
import codex_remote_mcp_windows_windows as windows_windows
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry
from simdorei_mcp_common.request_deadlines import RequestDeadlineExpired


@final
class _LaunchProcess:
    pid: int = 123

    def poll(self) -> int | None:
        return None

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return 0

    def terminate(self) -> None:
        return None

    def kill(self) -> None:
        return None


@final
class _BlockingProcess:
    pid: int = 123

    def __init__(self, *, never_exits: bool = False) -> None:
        self.killed: bool = False
        self.never_exits: bool = never_exits

    def poll(self) -> int | None:
        if self.killed and not self.never_exits:
            return 1
        return None

    def wait(self, timeout: float | None = None) -> int:
        if self.killed and not self.never_exits:
            return 1
        raise subprocess.TimeoutExpired("owned-process", timeout or 0.0)

    def terminate(self) -> None:
        return None

    def kill(self) -> None:
        self.killed = True


@final
class _OwnedJob:
    def __init__(self, *, fail: bool = False) -> None:
        self.terminated = False
        self.fail = fail

    def terminate_and_close(self, *, timeout_seconds: float = 5.0) -> None:
        _ = timeout_seconds
        self.terminated = True
        if self.fail:
            raise TimeoutError("job did not empty")


def _resolved() -> ResolvedWindow:
    return ResolvedWindow(
        entry=ComputerWindowEntry(
            window_id=42,
            title="Chrome",
            process_name="chrome.exe",
            left=0,
            top=0,
            width=800,
            height=600,
            active=True,
        ),
        identity=ComputerWindowIdentity(
            window_id=42,
            process_id=123,
            process_path=r"c:\program files\google\chrome\application\chrome.exe",
            title_digest="a" * 64,
            left=0,
            top=0,
            width=800,
            height=600,
        ),
    )


def test_chrome_profile_cleanup_retries_a_temporary_lock(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    attempts: list[str] = []

    def remove(directory: str) -> None:
        attempts.append(directory)
        if len(attempts) < 3:
            raise PermissionError("profile file is still locked")

    def no_sleep(_: float) -> None:
        return None

    monkeypatch.setattr("shutil.rmtree", remove)
    monkeypatch.setattr(time, "sleep", no_sleep)

    windows_launch.remove_temporary_profile("temporary-profile")

    assert attempts == ["temporary-profile"] * 3


def test_failed_launch_double_timeout_retains_cleanup_for_retry(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = _BlockingProcess(never_exits=True)
    cleanup_calls: list[str | None] = []
    monkeypatch.setattr(windows_launch, "_app_executable", lambda _: "chrome.exe")
    monkeypatch.setattr(
        windows_launch.tempfile,
        "mkdtemp",
        lambda **_: "profile",
    )
    monkeypatch.setattr(
        windows_launch.subprocess,
        "Popen",
        lambda *_args, **_kwargs: process,
    )
    monkeypatch.setattr(
        windows_launch,
        "create_kill_on_close_job_for_suspended_process",
        lambda _pid: _OwnedJob(fail=True),
    )

    def reject_window(_: windows_launch.OwnedProcess) -> ResolvedWindow:
        raise ComputerControlError("safe window did not appear")

    monkeypatch.setattr(windows_launch, "_wait_for_main_window", reject_window)
    monkeypatch.setattr(
        windows_launch,
        "remove_temporary_profile",
        cleanup_calls.append,
    )

    with pytest.raises(ComputerControlError) as captured:
        _ = windows_launch.launch_allowed_app("chrome")

    assert type(captured.value).__name__ == "ApplicationLaunchCleanupError"
    assert cleanup_calls == ["profile"]
    assert captured.value.reason == "Failed application cleanup must be retried."


def test_launch_assigns_suspended_process_to_owned_job(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = _LaunchProcess()
    job = _OwnedJob()
    creation_flags: list[int] = []
    monkeypatch.setattr(windows_launch, "_app_executable", lambda _: "chrome.exe")
    monkeypatch.setattr(
        windows_launch.tempfile,
        "mkdtemp",
        lambda **_: "profile",
    )

    def launch(_command: list[str], **kwargs: int) -> _LaunchProcess:
        creation_flags.append(kwargs["creationflags"])
        return process

    monkeypatch.setattr(windows_launch.subprocess, "Popen", launch)
    monkeypatch.setattr(
        windows_launch,
        "create_kill_on_close_job_for_suspended_process",
        lambda _pid: job,
    )
    monkeypatch.setattr(windows_launch, "_wait_for_main_window", lambda _: _resolved())

    launched = windows_launch.launch_allowed_app("chrome")
    windows_launch.stop_owned_process(
        launched.window.entry.window_id,
        launched.process,
        launched.window.identity.process_path,
    )

    assert creation_flags[0] & windows_launch.WINDOWS_CREATE_SUSPENDED
    assert job.terminated is True


def test_cancelled_launch_cleans_owned_process_tree(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process = _LaunchProcess()
    job = _OwnedJob()
    profiles: list[str | None] = []
    monkeypatch.setattr(windows_launch, "_app_executable", lambda _: "chrome.exe")
    monkeypatch.setattr(
        windows_launch.tempfile,
        "mkdtemp",
        lambda **_: "profile",
    )
    monkeypatch.setattr(
        windows_launch.subprocess,
        "Popen",
        lambda *_args, **_kwargs: process,
    )
    monkeypatch.setattr(
        windows_launch,
        "create_kill_on_close_job_for_suspended_process",
        lambda _pid: job,
    )
    monkeypatch.setattr(windows_launch, "remove_temporary_profile", profiles.append)

    with pytest.raises(RequestDeadlineExpired):
        _ = windows_launch.launch_allowed_app(
            "chrome",
            ensure_active=lambda: (_ for _ in ()).throw(
                RequestDeadlineExpired("cancelled")
            ),
        )

    assert job.terminated is True
    assert profiles == ["profile"]


def test_profile_cleanup_honors_expired_deadline() -> None:
    with pytest.raises(TimeoutError, match="Timed out removing"):
        windows_launch.remove_temporary_profile(
            "profile",
            deadline_monotonic=0.0,
        )


def test_process_identity_inspection_failure_is_not_reported_as_exit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def deny_inspection(_: int) -> str:
        raise ComputerControlError("Windows denied process identity inspection.")

    monkeypatch.setattr(windows_windows, "_process_path", deny_inspection)

    with pytest.raises(ComputerControlError, match="denied process identity"):
        _ = windows_windows.process_matches(123, r"c:\windows\notepad.exe")


def test_stop_uses_retained_process_handle_even_if_window_id_was_reused() -> None:
    process = _BlockingProcess()

    windows_launch.stop_owned_process(
        999,
        process,
        r"C:\unrelated\replacement.exe",
    )

    assert process.killed is True
    assert process.poll() == 1


def test_forced_stop_timeout_is_reported_as_a_computer_error() -> None:
    process = _BlockingProcess(never_exits=True)

    with pytest.raises(ComputerControlError, match="timed out"):
        windows_launch.stop_owned_process(
            42,
            process,
            r"C:\Windows\System32\notepad.exe",
        )


def test_forced_stop_uses_the_retained_process_handle() -> None:
    process = _BlockingProcess()
    windows_launch.stop_owned_process(
        42,
        process,
        r"C:\Windows\System32\notepad.exe",
    )

    assert process.killed is True
    assert process.poll() == 1

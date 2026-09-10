# pyright: reportAny=false
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import tempfile
import time
from collections.abc import Callable
from pathlib import Path
from typing import Final

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_launch_cleanup import (
    remove_temporary_profile,
)
from codex_remote_mcp_windows_launch_cleanup import (
    retry_failed_launch_cleanup as _retry_failed_launch_cleanup,
)
from codex_remote_mcp_windows_launch_types import (
    ApplicationLaunchCleanupError,
    FailedLaunchCleanup,
    JobOwnedProcess,
    LaunchedApplication,
    OwnedProcess,
    OwnedProcessTree,
)
from codex_remote_mcp_windows_native import USER32, EnumWindowsCallback
from codex_remote_mcp_windows_process_stop import stop_retained_process
from codex_remote_mcp_windows_windows import ResolvedWindow, resolve_allowed_window
from codex_windows_job import (
    WINDOWS_CREATE_SUSPENDED,
    create_kill_on_close_job_for_suspended_process,
)

LAUNCH_TIMEOUT_SECONDS: Final = 8.0
OWNED_PROCESS_CLOSE_SECONDS: Final = 1.0
OWNED_PROCESS_KILL_SECONDS: Final = 5.0


def launch_allowed_app(
    app: str,
    *,
    ensure_active: Callable[[], None] | None = None,
) -> LaunchedApplication:
    executable = _app_executable(app)
    temporary_profile = (
        tempfile.mkdtemp(prefix="simdorei-mcp-chrome-") if app == "chrome" else None
    )
    command = _launch_command(executable, app, temporary_profile)
    try:
        raw_process = subprocess.Popen(
            command,
            close_fds=True,
            creationflags=WINDOWS_CREATE_SUSPENDED,
        )
    except OSError as exc:
        _cleanup_failed_launch(
            FailedLaunchCleanup(app, None, temporary_profile),
            exc,
        )
        raise ComputerControlError(f"Could not launch {app}: {exc}") from exc
    try:
        job = create_kill_on_close_job_for_suspended_process(raw_process.pid)
        process: OwnedProcess = JobOwnedProcess(raw_process, job)
    except OSError as exc:
        _cleanup_failed_launch(
            FailedLaunchCleanup(app, raw_process, temporary_profile),
            exc,
        )
        raise ComputerControlError(
            f"Could not own the {app} process tree: {exc}"
        ) from exc
    try:
        if ensure_active is None:
            resolved = _wait_for_main_window(process)
        else:
            resolved = _wait_for_main_window(process, ensure_active=ensure_active)
    except BaseException as exc:
        _cleanup_failed_launch(
            FailedLaunchCleanup(app, process, temporary_profile),
            exc,
        )
        raise
    return LaunchedApplication(
        window=resolved,
        process=process,
        temporary_profile=temporary_profile,
    )


def _launch_command(
    executable: str,
    app: str,
    temporary_profile: str | None,
) -> list[str]:
    if app == "notepad":
        return [executable]
    if temporary_profile is None:
        raise ComputerControlError("Chrome requires an isolated temporary profile.")
    return [
        executable,
        f"--user-data-dir={temporary_profile}",
        "--guest",
        "--no-first-run",
        "--disable-sync",
        "--disable-background-mode",
        "--new-window",
        "about:blank",
    ]


def _wait_for_main_window(
    process: OwnedProcess,
    *,
    ensure_active: Callable[[], None] | None = None,
) -> ResolvedWindow:
    deadline = time.monotonic() + LAUNCH_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if ensure_active is not None:
            ensure_active()
        if process.poll() is not None:
            break
        for window_id in _visible_windows_for_process(process.pid):
            try:
                return resolve_allowed_window(window_id)
            except ComputerControlError:
                continue
        time.sleep(0.05)
    raise ComputerControlError(
        "The launched application did not open a safe window in time."
    )


def _visible_windows_for_process(process_id: int) -> tuple[int, ...]:
    windows: list[int] = []

    @EnumWindowsCallback
    def visit(hwnd: int, _lparam: int) -> bool:
        owner = wt.DWORD()
        USER32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if int(owner.value) == process_id and USER32.IsWindowVisible(hwnd):
            windows.append(int(hwnd))
        return True

    if not USER32.EnumWindows(visit, 0):
        raise ComputerControlError(
            "Windows could not inspect the launched application."
        )
    return tuple(windows)


def retry_failed_launch_cleanup(
    cleanup: FailedLaunchCleanup,
    *,
    deadline_monotonic: float | None = None,
) -> None:
    if deadline_monotonic is None:
        remove_profile = remove_temporary_profile
    else:
        def remove_profile(directory: str | None) -> None:
            remove_temporary_profile(
                directory,
                deadline_monotonic=deadline_monotonic,
            )
    _retry_failed_launch_cleanup(
        cleanup,
        remove_profile,
        deadline_monotonic=deadline_monotonic,
    )


def _cleanup_failed_launch(
    cleanup: FailedLaunchCleanup,
    launch_error: BaseException,
) -> None:
    try:
        retry_failed_launch_cleanup(cleanup)
    except ApplicationLaunchCleanupError as exc:
        raise exc from launch_error


def stop_owned_process(
    window_id: int,
    process: OwnedProcess,
    process_path: str,
    *,
    deadline_monotonic: float | None = None,
) -> None:
    _ = window_id, process_path
    if isinstance(process, OwnedProcessTree):
        try:
            timeout_seconds = (
                OWNED_PROCESS_KILL_SECONDS
                if deadline_monotonic is None
                else max(0.0, deadline_monotonic - time.monotonic())
            )
            process.terminate_tree_and_close(timeout_seconds=timeout_seconds)
        except (OSError, TimeoutError) as exc:
            raise ComputerControlError(
                "Stopping the owned application process tree failed."
            ) from exc
        return
    stop_retained_process(
        process,
        terminate_timeout_seconds=OWNED_PROCESS_CLOSE_SECONDS,
        kill_timeout_seconds=OWNED_PROCESS_KILL_SECONDS,
    )


def _app_executable(app: str) -> str:
    if app == "notepad":
        windows = os.environ.get("SYSTEMROOT")
        candidate = Path(windows) / "System32/notepad.exe" if windows else None
        if candidate is not None and candidate.is_file():
            return str(candidate)
        raise ComputerControlError("Windows Notepad is not installed in System32.")
    if app != "chrome":
        raise ComputerControlError(f"Application is not allowlisted: {app}")
    candidates = tuple(
        base / "Google/Chrome/Application/chrome.exe"
        for variable in ("PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA")
        if (value := os.environ.get(variable))
        for base in (Path(value),)
    )
    for candidate in candidates:
        if candidate.is_file():
            return str(candidate)
    raise ComputerControlError("Google Chrome is not installed in a standard location.")

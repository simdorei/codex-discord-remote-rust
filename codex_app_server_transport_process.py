from __future__ import annotations

import os
import subprocess
from collections.abc import Sequence
from typing import IO, Callable, Protocol, final

from codex_app_server_transport_replies import CodexAppServerTransportError
from codex_windows_job import (
    WINDOWS_CREATE_SUSPENDED,
    WindowsKillOnCloseJob,
    create_kill_on_close_job_for_suspended_process,
)


LogFunc = Callable[[str], None]
_GRACEFUL_CLOSE_TIMEOUT_SEC = 1.5
_FORCED_CLOSE_TIMEOUT_SEC = 5.0


class ResidentProcess(Protocol):
    @property
    def pid(self) -> int: ...

    @property
    def stdin(self) -> IO[str] | None: ...

    @property
    def stdout(self) -> IO[str] | None: ...

    @property
    def stderr(self) -> IO[str] | None: ...

    def poll(self) -> int | None: ...

    def wait(self, timeout: float | None = None) -> int: ...

    def terminate(self) -> None: ...

    def kill(self) -> None: ...


@final
class OwnedResidentProcess:
    """Popen facade that retains the OS ownership handle for its full tree."""

    __slots__ = ("_job", "_process")

    def __init__(
        self,
        process: subprocess.Popen[str],
        job: WindowsKillOnCloseJob | None,
    ) -> None:
        self._process = process
        self._job = job

    @property
    def pid(self) -> int:
        return self._process.pid

    @property
    def stdin(self) -> IO[str] | None:
        return self._process.stdin

    @property
    def stdout(self) -> IO[str] | None:
        return self._process.stdout

    @property
    def stderr(self) -> IO[str] | None:
        return self._process.stderr

    def poll(self) -> int | None:
        return self._process.poll()

    def wait(self, timeout: float | None = None) -> int:
        return self._process.wait(timeout=timeout)

    def terminate(self) -> None:
        self._process.terminate()

    def kill(self) -> None:
        self._process.kill()

    def terminate_owned_tree(self) -> None:
        if self._job is None:
            if self.poll() is None:
                self.terminate()
            return
        self._job.terminate_and_close()


def start_owned_app_server_command(command: Sequence[str]) -> OwnedResidentProcess:
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    if os.name == "nt":
        creationflags |= WINDOWS_CREATE_SUSPENDED
    process = subprocess.Popen(
        list(command),
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
        creationflags=creationflags,
    )
    try:
        job = (
            create_kill_on_close_job_for_suspended_process(process.pid)
            if os.name == "nt"
            else None
        )
    except OSError:
        process.kill()
        _ = process.wait(timeout=_FORCED_CLOSE_TIMEOUT_SEC)
        _close_process_pipes(process)
        raise
    return OwnedResidentProcess(process, job)


def start_resident_app_server_process(executable: str) -> ResidentProcess:
    try:
        return start_owned_app_server_command([executable, "app-server"])
    except OSError as exc:
        raise CodexAppServerTransportError(
            "Failed to start resident Codex app-server. " + f"executable={executable!r}"
        ) from exc


def has_resident_app_server_stdio(process: ResidentProcess) -> bool:
    return process.stdin is not None and process.stdout is not None


def close_resident_app_server_process(process: ResidentProcess, log: LogFunc) -> None:
    stdin = process.stdin
    if stdin is not None and not stdin.closed:
        try:
            stdin.close()
        except OSError as exc:
            log(f"app_server_transport_stdin_close_failed error_type={type(exc).__name__} error={exc}")

    graceful = _wait_for_exit(process, timeout_sec=_GRACEFUL_CLOSE_TIMEOUT_SEC, log=log)
    if isinstance(process, OwnedResidentProcess):
        try:
            process.terminate_owned_tree()
        except (OSError, TimeoutError) as exc:
            log(
                "app_server_transport_job_terminate_failed "
                + f"error_type={type(exc).__name__} error={exc}"
            )
            raise
        if process.poll() is None:
            try:
                _ = process.wait(timeout=_FORCED_CLOSE_TIMEOUT_SEC)
            except subprocess.TimeoutExpired as exc:
                raise TimeoutError(
                    f"Timed out waiting for owned app-server PID {process.pid} to exit."
                ) from exc
    elif not graceful and process.poll() is None:
        _terminate_unowned_process(process, log)
    _close_process_pipes(process)


def _wait_for_exit(
    process: ResidentProcess,
    *,
    timeout_sec: float,
    log: LogFunc,
) -> bool:
    if process.poll() is not None:
        return True
    try:
        _ = process.wait(timeout=timeout_sec)
    except subprocess.TimeoutExpired:
        log(f"app_server_transport_graceful_close_timed_out pid={process.pid}")
        return False
    except (OSError, RuntimeError) as exc:
        log(
            "app_server_transport_graceful_wait_failed "
            + f"error_type={type(exc).__name__} error={exc}"
        )
        log(f"app_server_transport_graceful_close_timed_out pid={process.pid}")
        return False
    return True


def _terminate_unowned_process(process: ResidentProcess, log: LogFunc) -> None:
    try:
        process.terminate()
        _ = process.wait(timeout=_FORCED_CLOSE_TIMEOUT_SEC)
        return
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        log(
            "app_server_transport_terminate_failed "
            + f"error_type={type(exc).__name__} error={exc}"
        )
    try:
        process.kill()
        _ = process.wait(timeout=_FORCED_CLOSE_TIMEOUT_SEC)
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
        log(
            "app_server_transport_kill_failed "
            + f"error_type={type(exc).__name__} error={exc}"
        )


def _close_process_pipes(process: ResidentProcess) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None and not pipe.closed:
            pipe.close()

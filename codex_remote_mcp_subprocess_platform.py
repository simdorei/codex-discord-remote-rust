from __future__ import annotations

import os
import signal
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import Final

from codex_windows_job import (
    WINDOWS_CREATE_SUSPENDED,
    WindowsKillOnCloseJob,
)

_CLEANUP_TIMEOUT_SECONDS: Final = 5.0


def start_owned_process(
    args: tuple[str, ...],
    *,
    cwd: Path,
    env: Mapping[str, str],
) -> subprocess.Popen[bytes]:
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    if os.name == "nt":
        creationflags |= WINDOWS_CREATE_SUSPENDED
    return subprocess.Popen(
        args,
        cwd=cwd,
        env=dict(env),
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        creationflags=creationflags,
        start_new_session=os.name != "nt",
    )


def terminate_owned_tree(
    process: subprocess.Popen[bytes],
    job: WindowsKillOnCloseJob | None,
) -> BaseException | None:
    error: BaseException | None = None
    try:
        if job is not None:
            job.terminate_and_close()
        elif os.name != "nt":
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    except BaseException as exc:  # noqa: BLE001 - returned without masking primary failure.
        error = exc
    if process.poll() is None:
        try:
            process.kill()
        except OSError as exc:
            error = error or exc
    try:
        _ = process.wait(timeout=_CLEANUP_TIMEOUT_SECONDS)
    except (OSError, subprocess.TimeoutExpired) as exc:
        error = error or exc
    return error


def kill_direct_process(process: subprocess.Popen[bytes]) -> None:
    try:
        process.kill()
        _ = process.wait(timeout=_CLEANUP_TIMEOUT_SECONDS)
    except (OSError, subprocess.TimeoutExpired):
        pass


def close_process_pipes(process: subprocess.Popen[bytes]) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None and not pipe.closed:
            pipe.close()


__all__ = [
    "close_process_pipes",
    "kill_direct_process",
    "start_owned_process",
    "terminate_owned_tree",
]

from __future__ import annotations

import subprocess

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_launch_types import OwnedProcess


def stop_retained_process(
    process: OwnedProcess,
    *,
    terminate_timeout_seconds: float,
    kill_timeout_seconds: float,
) -> None:
    """Stop only the process represented by the retained OS process handle."""
    try:
        if process.poll() is not None:
            return
        process.terminate()
        try:
            _ = process.wait(timeout=terminate_timeout_seconds)
            return
        except subprocess.TimeoutExpired:
            process.kill()
            _ = process.wait(timeout=kill_timeout_seconds)
    except subprocess.TimeoutExpired as exc:
        raise ComputerControlError(
            "Stopping the session-owned application timed out."
        ) from exc
    except OSError as exc:
        try:
            exited = process.poll() is not None
        except OSError:
            exited = False
        if exited:
            return
        raise ComputerControlError(
            "Windows could not stop the session-owned application."
        ) from exc
    if process.poll() is None:
        raise ComputerControlError("The session-owned application is still running.")

from __future__ import annotations

import subprocess
from collections.abc import Callable

from codex_desktop_bridge_sidecar_process import SidecarProcess, StartProcess
from codex_desktop_bridge_sidecar_protocol import CodexSidecarStartupError


def start_app_server_process(
    executable_resolver: Callable[[], str],
    start_process: StartProcess,
    *,
    app_server_exe_env: str,
) -> SidecarProcess:
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    executable = executable_resolver()
    try:
        return start_process(
            [executable, "app-server"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            bufsize=1,
            creationflags=creationflags,
        )
    except OSError as exc:
        raise CodexSidecarStartupError(
            "Failed to start the Codex app-server sidecar. "
            + f"executable={executable!r}. "
            + f"Run `doctor` or set {app_server_exe_env} to the app-server codex.exe path."
        ) from exc


def ensure_stdio_available(process: SidecarProcess) -> None:
    if process.stdin is None or process.stdout is None:
        _ = close_sidecar_process(process)
        raise CodexSidecarStartupError("Failed to start the Codex app-server sidecar.")


def close_sidecar_process(process: SidecarProcess) -> tuple[OSError, ...]:
    close_errors: list[OSError] = []
    stdin = process.stdin
    if stdin is not None and not stdin.closed:
        try:
            stdin.close()
        except OSError as exc:
            close_errors.append(exc)

    if process.poll() is None:
        try:
            _ = process.terminate()
            _ = process.wait(timeout=1.5)
        except (OSError, subprocess.TimeoutExpired):
            try:
                _ = process.kill()
            except OSError as exc:
                close_errors.append(exc)
    return tuple(close_errors)

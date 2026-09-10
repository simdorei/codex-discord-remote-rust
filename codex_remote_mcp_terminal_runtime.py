from __future__ import annotations

import os
import shutil
import time
from pathlib import Path
from typing import Final, final

from codex_remote_mcp_subprocess import CancellationSignal
from simdorei_mcp_common.terminal_protocol import TerminalCwdScope, TerminalShell

_SECRET_ENV_MARKERS: Final = (
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "COOKIE",
    "CREDENTIAL",
    "OTP",
)


@final
class CombinedCancellation:
    def __init__(self, *signals: CancellationSignal | None) -> None:
        self._signals: tuple[CancellationSignal, ...] = tuple(
            signal for signal in signals if signal is not None
        )

    def is_set(self) -> bool:
        return any(signal.is_set() for signal in self._signals)

    def wait(self, timeout: float | None = None) -> bool:
        deadline = None if timeout is None else time.monotonic() + timeout
        while not self.is_set():
            remaining = None if deadline is None else deadline - time.monotonic()
            if remaining is not None and remaining <= 0:
                return False
            time.sleep(0.02 if remaining is None else min(0.02, remaining))
        return True


def resolve_terminal_cwd(current: Path, requested: str | None) -> Path:
    if requested is None:
        return current
    candidate = Path(requested)
    resolved = (candidate if candidate.is_absolute() else current / candidate).resolve(
        strict=True
    )
    if not resolved.is_dir():
        raise NotADirectoryError(requested)
    return resolved


def terminal_cwd_scope(root: Path, cwd: Path) -> TerminalCwdScope:
    try:
        relative = cwd.relative_to(root)
    except ValueError:
        return "external_absolute"
    return "project_root" if relative == Path() else "project_relative"


def terminal_shell_argv(
    shell: TerminalShell,
    command: str,
) -> tuple[TerminalShell, tuple[str, ...]]:
    selected: TerminalShell = (
        "powershell" if shell == "auto" and os.name == "nt" else shell
    )
    if selected == "auto":
        selected = "sh"
    if selected == "powershell":
        executable = shutil.which("pwsh") or shutil.which("powershell")
        arguments = ("-NoLogo", "-NoProfile", "-NonInteractive", "-Command")
    elif selected == "cmd":
        executable = os.environ.get("ComSpec") or shutil.which("cmd")
        arguments = ("/D", "/S", "/C")
    else:
        executable = shutil.which(selected)
        arguments = ("-c",)
    if executable is None:
        raise FileNotFoundError(f"requested terminal shell is unavailable: {selected}")
    return selected, (executable, *arguments, command)


def inherited_terminal_environment() -> dict[str, str]:
    return {
        key: value
        for key, value in os.environ.items()
        if not any(marker in key.upper() for marker in _SECRET_ENV_MARKERS)
    }


__all__ = [
    "CombinedCancellation",
    "inherited_terminal_environment",
    "resolve_terminal_cwd",
    "terminal_cwd_scope",
    "terminal_shell_argv",
]

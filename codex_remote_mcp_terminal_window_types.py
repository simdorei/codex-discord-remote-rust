from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_remote_mcp_windows_launch_types import OwnedProcess
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowEntry,
    TerminalWindowShell,
)


@dataclass(frozen=True, slots=True)
class OwnedTerminalWindow:
    entry: TerminalWindowEntry
    process: OwnedProcess
    window_process_id: int | None = None


class TerminalWindowBackend(Protocol):
    def require_supported(self) -> None: ...

    def open(
        self,
        terminal_window_id: str,
        shell: TerminalWindowShell,
        cwd: Path,
        title: str,
    ) -> OwnedTerminalWindow: ...

    def inspect(
        self,
        window: OwnedTerminalWindow,
    ) -> TerminalWindowEntry | None: ...

    def close(self, window: OwnedTerminalWindow) -> None: ...


__all__ = ["OwnedTerminalWindow", "TerminalWindowBackend"]

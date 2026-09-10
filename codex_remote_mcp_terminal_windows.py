from __future__ import annotations

import os
import secrets
import threading
from pathlib import Path
from typing import final, override

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_runtime import resolve_terminal_cwd
from codex_remote_mcp_terminal_window_interaction_types import (
    TerminalWindowInteractionBackend,
)
from codex_remote_mcp_terminal_window_interactions import (
    TerminalWindowInteractionController,
    default_terminal_window_interaction_backend,
)
from codex_remote_mcp_terminal_window_types import (
    OwnedTerminalWindow,
    TerminalWindowBackend,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseOutput,
    TerminalWindowCloseRequest,
    TerminalWindowListOutput,
    TerminalWindowOpenOutput,
    TerminalWindowOpenRequest,
    TerminalWindowEntry,
    TerminalWindowShell,
)
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowActionOutput,
    TerminalWindowActivateRequest,
    TerminalWindowCaptureOutput,
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowTypeRequest,
)


@final
class UnsupportedTerminalWindowBackend(TerminalWindowBackend):
    @override
    def require_supported(self) -> None:
        raise TerminalExecutionError(
            "visible terminal windows are supported only on Windows"
        )

    @override
    def open(
        self,
        terminal_window_id: str,
        shell: TerminalWindowShell,
        cwd: Path,
        title: str,
    ) -> OwnedTerminalWindow:
        _ = terminal_window_id, shell, cwd, title
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def inspect(self, window: OwnedTerminalWindow) -> TerminalWindowEntry | None:
        _ = window
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def close(self, window: OwnedTerminalWindow) -> None:
        _ = window
        self.require_supported()


@final
class TerminalWindowManager:
    """Own visible terminal windows for exactly one selected MCP session."""

    def __init__(
        self,
        root: Path,
        *,
        backend: TerminalWindowBackend | None = None,
        interaction_backend: TerminalWindowInteractionBackend | None = None,
    ) -> None:
        self._root = root.resolve(strict=True)
        self._backend = backend or _default_backend()
        self._lock = threading.RLock()
        self._windows: dict[str, OwnedTerminalWindow] = {}
        self._interactions = TerminalWindowInteractionController(
            self._lock,
            self._windows,
            self._backend,
            interaction_backend or default_terminal_window_interaction_backend(),
        )
        self._closed = False

    def open(self, request: TerminalWindowOpenRequest) -> TerminalWindowOpenOutput:
        cwd = resolve_terminal_cwd(self._root, request.cwd)
        with self._lock:
            self._require_open()
            self._backend.require_supported()
            terminal_window_id = f"termwin_{secrets.token_hex(8)}"
            title = f"Codex Pro Terminal {terminal_window_id} " + secrets.token_hex(8)
            owned = self._backend.open(
                terminal_window_id,
                request.shell,
                cwd,
                title,
            )
            self._windows[terminal_window_id] = owned
            return TerminalWindowOpenOutput(window=owned.entry)

    def list(self) -> TerminalWindowListOutput:
        with self._lock:
            self._require_open()
            self._backend.require_supported()
            entries = self._prune_and_list_locked()
            return TerminalWindowListOutput(
                windows=tuple(
                    sorted(entries, key=lambda entry: entry.terminal_window_id)
                )
            )

    def close(
        self,
        request: TerminalWindowCloseRequest,
    ) -> TerminalWindowCloseOutput:
        with self._lock:
            self._require_open()
            self._backend.require_supported()
            owned = self._windows.get(request.terminal_window_id)
            if owned is None:
                raise TerminalExecutionError(
                    "terminal window does not belong to this session"
                )
            try:
                self._backend.close(owned)
            finally:
                self._interactions.drop(request.terminal_window_id)
            del self._windows[request.terminal_window_id]
            return TerminalWindowCloseOutput(
                terminal_window_id=request.terminal_window_id
            )

    def close_all(self) -> None:
        with self._lock:
            self._closed = True
            failures: list[Exception] = []
            for terminal_window_id, owned in tuple(self._windows.items()):
                try:
                    self._backend.close(owned)
                except Exception as exc:  # noqa: BLE001 - finish every cleanup.
                    failures.append(exc)
                else:
                    del self._windows[terminal_window_id]
                finally:
                    self._interactions.drop(terminal_window_id)
            self._interactions.clear()
            if failures:
                raise failures[0]

    def capture(
        self,
        request: TerminalWindowCaptureRequest,
    ) -> TerminalWindowCaptureOutput:
        with self._lock:
            self._require_open()
            return self._interactions.capture(request)

    def activate(
        self,
        request: TerminalWindowActivateRequest,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            self._require_open()
            return self._interactions.activate(request)

    def type_text(
        self,
        request: TerminalWindowTypeRequest,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            self._require_open()
            return self._interactions.type_text(request)

    def press_keys(
        self,
        request: TerminalWindowKeysRequest,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            self._require_open()
            return self._interactions.press_keys(request)

    def interrupt(
        self,
        request: TerminalWindowInterruptRequest,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            self._require_open()
            return self._interactions.interrupt(request)

    def _prune_and_list_locked(self) -> list[TerminalWindowEntry]:
        entries: list[TerminalWindowEntry] = []
        for terminal_window_id, owned in tuple(self._windows.items()):
            entry = self._backend.inspect(owned)
            if entry is not None:
                entries.append(entry)
                continue
            try:
                self._backend.close(owned)
            finally:
                self._interactions.drop(terminal_window_id)
            del self._windows[terminal_window_id]
        return entries

    def _require_open(self) -> None:
        if self._closed:
            raise TerminalExecutionError("terminal window session is closed")


def _default_backend() -> TerminalWindowBackend:
    if os.name != "nt":
        return UnsupportedTerminalWindowBackend()
    from codex_remote_mcp_terminal_window_windows import (
        WindowsTerminalWindowBackend,
    )

    return WindowsTerminalWindowBackend()


__all__ = [
    "TerminalWindowManager",
    "UnsupportedTerminalWindowBackend",
]

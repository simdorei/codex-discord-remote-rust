from __future__ import annotations

import threading
from dataclasses import dataclass
from pathlib import Path
from typing import final

from codex_remote_mcp_terminal_engine import TerminalExecutionEngine
from codex_remote_mcp_terminal_windows import TerminalWindowManager


@dataclass(slots=True)
class _SessionResources:
    root: Path
    session_id: str
    engine: TerminalExecutionEngine
    windows: TerminalWindowManager

    def close(self) -> None:
        failures: list[Exception] = []
        for close in (self.windows.close_all, self.engine.close):
            try:
                close()
            except Exception as exc:  # noqa: BLE001 - finish both cleanups.
                failures.append(exc)
        if failures:
            raise failures[0]


@final
class TerminalSessionRegistry:
    """Own one terminal engine per bound thread and ChatGPT session."""

    def __init__(self) -> None:
        self._lock: threading.RLock = threading.RLock()
        self._resources: dict[str, _SessionResources] = {}
        self._retired: list[_SessionResources] = []

    def for_session(
        self,
        thread_id: str,
        root: Path,
        session_id: str,
    ) -> TerminalExecutionEngine:
        return self._for_session(thread_id, root, session_id).engine

    def windows_for_session(
        self,
        thread_id: str,
        root: Path,
        session_id: str,
    ) -> TerminalWindowManager:
        return self._for_session(thread_id, root, session_id).windows

    def _for_session(
        self,
        thread_id: str,
        root: Path,
        session_id: str,
    ) -> _SessionResources:
        with self._lock:
            self._retry_retired_locked()
            resolved_root = root.resolve(strict=True)
            current = self._resources.get(thread_id)
            if (
                current is not None
                and current.session_id == session_id
                and current.root == resolved_root
            ):
                return current
            if current is not None:
                del self._resources[thread_id]
                self._close_or_retire_locked(current)
            created = _SessionResources(
                root=resolved_root,
                session_id=session_id,
                engine=TerminalExecutionEngine(root, session_id=session_id),
                windows=TerminalWindowManager(root),
            )
            self._resources[thread_id] = created
            return created

    def close_thread(self, thread_id: str) -> None:
        with self._lock:
            self._retry_retired_locked()
            current = self._resources.pop(thread_id, None)
            if current is not None:
                self._close_or_retire_locked(current)

    def close_all(self) -> None:
        with self._lock:
            resources = (*self._resources.values(), *self._retired)
            self._resources.clear()
            self._retired.clear()
            failures: list[Exception] = []
            for resource in resources:
                try:
                    resource.close()
                except Exception as exc:  # noqa: BLE001 - finish every owned cleanup.
                    self._retired.append(resource)
                    failures.append(exc)
            if failures:
                raise failures[0]

    def _close_or_retire_locked(self, resource: _SessionResources) -> None:
        try:
            resource.close()
        except Exception:
            self._retired.append(resource)
            raise

    def _retry_retired_locked(self) -> None:
        retired = tuple(self._retired)
        self._retired.clear()
        failures: list[Exception] = []
        for resource in retired:
            try:
                resource.close()
            except Exception as exc:  # noqa: BLE001 - retain retryable cleanup.
                self._retired.append(resource)
                failures.append(exc)
        if failures:
            raise failures[0]


__all__ = ["TerminalSessionRegistry"]

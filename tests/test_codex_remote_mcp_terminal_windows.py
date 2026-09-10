from __future__ import annotations

from pathlib import Path
from typing import final, override

import pytest

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_types import (
    OwnedTerminalWindow,
    TerminalWindowBackend,
)
from codex_remote_mcp_terminal_windows import (
    TerminalWindowManager,
    UnsupportedTerminalWindowBackend,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseRequest,
    TerminalWindowEntry,
    TerminalWindowOpenRequest,
    TerminalWindowShell,
)


@final
class FakeProcess:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.exit_code: int | None = None

    def poll(self) -> int | None:
        return self.exit_code

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return self.exit_code or 0

    def terminate(self) -> None:
        self.exit_code = 0

    def kill(self) -> None:
        self.exit_code = 1


@final
class FakeBackend(TerminalWindowBackend):
    def __init__(self) -> None:
        self.next_pid = 100
        self.closed: list[str] = []
        self.stale: set[str] = set()
        self.fail_open = False
        self.fail_close_once = False

    @override
    def require_supported(self) -> None:
        return None

    @override
    def open(
        self,
        terminal_window_id: str,
        shell: TerminalWindowShell,
        cwd: Path,
        title: str,
    ) -> OwnedTerminalWindow:
        if self.fail_open:
            raise TerminalExecutionError("synthetic launch failure")
        self.next_pid += 1
        return OwnedTerminalWindow(
            entry=TerminalWindowEntry(
                terminal_window_id=terminal_window_id,
                window_id=self.next_pid + 1_000,
                process_id=self.next_pid,
                shell=shell,
                cwd=str(cwd),
                title=title,
            ),
            process=FakeProcess(self.next_pid),
        )

    @override
    def inspect(self, window: OwnedTerminalWindow) -> TerminalWindowEntry | None:
        if window.entry.terminal_window_id in self.stale:
            return None
        return window.entry

    @override
    def close(self, window: OwnedTerminalWindow) -> None:
        if self.fail_close_once:
            self.fail_close_once = False
            raise TerminalExecutionError("synthetic close failure")
        window.process.terminate()
        self.closed.append(window.entry.terminal_window_id)


def test_manager_opens_lists_and_closes_distinct_window_ids(tmp_path: Path) -> None:
    backend = FakeBackend()
    manager = TerminalWindowManager(tmp_path, backend=backend)

    powershell = manager.open(TerminalWindowOpenRequest(shell="powershell"))
    cmd = manager.open(TerminalWindowOpenRequest(shell="cmd", cwd=str(tmp_path)))
    listed = manager.list()

    assert powershell.window.terminal_window_id.startswith("termwin_")
    assert cmd.window.terminal_window_id != powershell.window.terminal_window_id
    assert {window.shell for window in listed.windows} == {"powershell", "cmd"}

    closed = manager.close(
        TerminalWindowCloseRequest(
            terminal_window_id=powershell.window.terminal_window_id
        )
    )
    assert closed.closed is True
    assert tuple(window.shell for window in manager.list().windows) == ("cmd",)
    manager.close_all()
    assert set(backend.closed) == {
        powershell.window.terminal_window_id,
        cmd.window.terminal_window_id,
    }


def test_manager_prunes_a_manually_closed_window(tmp_path: Path) -> None:
    backend = FakeBackend()
    manager = TerminalWindowManager(tmp_path, backend=backend)
    opened = manager.open(TerminalWindowOpenRequest())
    backend.stale.add(opened.window.terminal_window_id)

    assert manager.list().windows == ()
    assert backend.closed == [opened.window.terminal_window_id]


def test_manager_keeps_failed_cleanup_for_retry(tmp_path: Path) -> None:
    backend = FakeBackend()
    manager = TerminalWindowManager(tmp_path, backend=backend)
    opened = manager.open(TerminalWindowOpenRequest())
    backend.fail_close_once = True

    with pytest.raises(TerminalExecutionError, match="synthetic close failure"):
        manager.close_all()
    manager.close_all()

    assert backend.closed == [opened.window.terminal_window_id]


def test_manager_does_not_register_a_failed_launch(tmp_path: Path) -> None:
    backend = FakeBackend()
    manager = TerminalWindowManager(tmp_path, backend=backend)
    backend.fail_open = True

    with pytest.raises(TerminalExecutionError, match="synthetic launch failure"):
        _ = manager.open(TerminalWindowOpenRequest())
    backend.fail_open = False
    assert manager.list().windows == ()


def test_non_windows_backend_fails_explicitly(tmp_path: Path) -> None:
    manager = TerminalWindowManager(
        tmp_path,
        backend=UnsupportedTerminalWindowBackend(),
    )

    with pytest.raises(TerminalExecutionError, match="supported only on Windows"):
        _ = manager.open(TerminalWindowOpenRequest())
    with pytest.raises(TerminalExecutionError, match="supported only on Windows"):
        _ = manager.list()

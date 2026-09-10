from __future__ import annotations

import os
import time
from pathlib import Path

import pytest

from codex_remote_mcp_terminal_sessions import TerminalSessionRegistry
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from codex_remote_mcp_windows_launch_types import OwnedProcess
from codex_remote_mcp_windows_native import USER32
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseRequest,
    TerminalWindowOpenRequest,
)

pytestmark = pytest.mark.skipif(os.name != "nt", reason="Windows-only real window QA")
_WM_CLOSE = 0x0010


def _wait_exited(process: OwnedProcess) -> None:
    deadline = time.monotonic() + 10
    while process.poll() is None and time.monotonic() < deadline:
        time.sleep(0.05)
    assert process.poll() is not None


def test_real_windows_backend_opens_lists_and_closes_both_shells(
    tmp_path: Path,
) -> None:
    manager = TerminalWindowManager(tmp_path)
    try:
        powershell = manager.open(TerminalWindowOpenRequest(shell="powershell"))
        cmd = manager.open(TerminalWindowOpenRequest(shell="cmd"))
        powershell_process = manager._windows[  # pyright: ignore[reportPrivateUsage]
            powershell.window.terminal_window_id
        ].process
        cmd_process = manager._windows[  # pyright: ignore[reportPrivateUsage]
            cmd.window.terminal_window_id
        ].process

        listed = manager.list()
        assert {window.shell for window in listed.windows} == {"powershell", "cmd"}
        assert USER32.IsWindow(powershell.window.window_id)
        assert USER32.IsWindow(cmd.window.window_id)

        _ = manager.close(
            TerminalWindowCloseRequest(
                terminal_window_id=powershell.window.terminal_window_id
            )
        )
        _wait_exited(powershell_process)
        manager.close_all()
        _wait_exited(cmd_process)
    finally:
        manager.close_all()


def test_real_windows_backend_prunes_a_manually_closed_console(tmp_path: Path) -> None:
    manager = TerminalWindowManager(tmp_path)
    try:
        opened = manager.open(TerminalWindowOpenRequest(shell="cmd"))
        assert USER32.PostMessageW(opened.window.window_id, _WM_CLOSE, 0, 0)

        deadline = time.monotonic() + 10
        listed = manager.list()
        while listed.windows and time.monotonic() < deadline:
            time.sleep(0.05)
            listed = manager.list()

        assert listed.windows == ()
    finally:
        manager.close_all()


def test_session_registry_close_kills_every_owned_window(tmp_path: Path) -> None:
    registry = TerminalSessionRegistry()
    manager = registry.windows_for_session(
        "thread-a",
        tmp_path,
        "session-a",
    )
    opened = manager.open(TerminalWindowOpenRequest())
    process = manager._windows[  # pyright: ignore[reportPrivateUsage]
        opened.window.terminal_window_id
    ].process

    registry.close_thread("thread-a")

    _wait_exited(process)

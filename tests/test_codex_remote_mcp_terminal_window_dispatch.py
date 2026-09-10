from __future__ import annotations

import os
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from codex_remote_mcp_windows_native import USER32
from simdorei_mcp_common.messages import (
    OperationErrorResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    RequestId,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseOutput,
    TerminalWindowCloseRequest,
    TerminalWindowListOutput,
    TerminalWindowListRequest,
    TerminalWindowOpenOutput,
    TerminalWindowOpenRequest,
    TerminalWindowRequest,
)
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)

pytestmark = pytest.mark.skipif(os.name != "nt", reason="Windows-only real window QA")


def test_dispatch_round_trips_visible_terminal_window_lifecycle(
    tmp_path: Path,
) -> None:
    dispatcher = _dispatcher(tmp_path)
    try:
        opened = _open_output(
            dispatcher.execute(
                _command(
                    "window-open",
                    TerminalWindowOpenRequest(shell="powershell"),
                )
            )
        )
        listed = _list_output(
            dispatcher.execute(_command("window-list", TerminalWindowListRequest()))
        )
        closed = _close_output(
            dispatcher.execute(
                _command(
                    "window-close",
                    TerminalWindowCloseRequest(
                        terminal_window_id=opened.window.terminal_window_id
                    ),
                )
            )
        )

        assert listed.windows == (opened.window,)
        assert closed.terminal_window_id == opened.window.terminal_window_id
        _wait_window_closed(opened.window.window_id)
    finally:
        dispatcher.retire_computer_sessions()


def test_session_replacement_closes_owned_terminal_windows(tmp_path: Path) -> None:
    dispatcher = _dispatcher(tmp_path)
    opened = _open_output(
        dispatcher.execute(_command("window-open-old", TerminalWindowOpenRequest()))
    )
    try:
        replaced = dispatcher.execute(
            ProjectSessionCommand(
                request_id=RequestId("window-session-replace"),
                thread_id="thread-a",
                computer_session_id="terminal-window-session-two",
                computer_session_generation=2,
            )
        )
        _wait_window_closed(opened.window.window_id)

        stale = dispatcher.execute(
            _command("window-list-stale", TerminalWindowListRequest())
        )
        fresh = dispatcher.execute(
            _command(
                "window-list-fresh",
                TerminalWindowListRequest(),
                session_id="terminal-window-session-two",
            )
        )

        assert isinstance(replaced, ProjectSessionResult)
        assert isinstance(stale, OperationErrorResult)
        assert stale.error_code == "computer_control"
        assert _list_output(fresh).windows == ()
    finally:
        dispatcher.retire_computer_sessions()


def _dispatcher(root: Path) -> LocalProjectDispatcher:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        root,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    activate_test_session(dispatcher)
    return dispatcher


def _command(
    request_id: str,
    operation: TerminalWindowRequest,
    *,
    session_id: str = TEST_PROJECT_SESSION_ID,
) -> ProjectOperationCommand:
    return ProjectOperationCommand(
        request_id=RequestId(request_id),
        thread_id="thread-a",
        computer_session_id=session_id,
        operation=operation,
    )


def _open_output(result: object) -> TerminalWindowOpenOutput:
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, TerminalWindowOpenOutput)
    return result.output


def _list_output(result: object) -> TerminalWindowListOutput:
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, TerminalWindowListOutput)
    return result.output


def _close_output(result: object) -> TerminalWindowCloseOutput:
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, TerminalWindowCloseOutput)
    return result.output


def _wait_window_closed(window_id: int) -> None:
    deadline = time.monotonic() + 10
    while USER32.IsWindow(window_id) and time.monotonic() < deadline:
        time.sleep(0.05)
    assert not USER32.IsWindow(window_id)

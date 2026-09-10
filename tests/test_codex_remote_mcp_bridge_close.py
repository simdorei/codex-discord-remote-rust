from __future__ import annotations

import threading
from pathlib import Path

from codex_remote_mcp_bridge import RemoteMcpBridge
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    RequestId,
)
from simdorei_mcp_common.operation_requests import ComputerListWindowsRequest
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)
from tests.test_codex_remote_mcp_bridge import FakeConnector, FakeSocket, _config


def test_close_does_not_wait_forever_for_a_project_execution_lock(
    tmp_path: Path,
) -> None:
    socket = FakeSocket()
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    bridge = RemoteMcpBridge(
        _config(),
        connector=FakeConnector(socket),
        dispatcher=dispatcher,
        close_timeout_seconds=0.05,
        log=lambda _: None,
    )
    _ = bridge.register_project(
        "thread-1",
        "codex-pro-project-close-deadline",
        tmp_path,
    )
    project = dispatcher._state.binding("thread-1")
    assert project is not None
    activated = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId("activate-close-deadline"),
            thread_id="thread-1",
            computer_session_id="close-deadline-session",
            computer_session_generation=1,
        )
    )
    listed = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("create-close-controller"),
            thread_id="thread-1",
            computer_session_id="close-deadline-session",
            operation=ComputerListWindowsRequest(),
        )
    )
    assert isinstance(activated, ProjectSessionResult)
    assert isinstance(listed, ProjectOperationResult)
    controller = dispatcher._state._computers["thread-1"]
    lock_held = threading.Event()
    release_lock = threading.Event()

    def hold_execution_lock() -> None:
        with project.execution_lock:
            lock_held.set()
            assert release_lock.wait(timeout=5)

    holder = threading.Thread(target=hold_execution_lock)
    holder.start()
    assert lock_held.wait(timeout=2)
    close_errors: list[RemoteMcpBridgeError] = []
    close_finished = threading.Event()

    def close_bridge() -> None:
        try:
            bridge.close()
        except RemoteMcpBridgeError as exc:
            close_errors.append(exc)
        finally:
            close_finished.set()

    closer = threading.Thread(target=close_bridge)
    closer.start()
    try:
        assert close_finished.wait(timeout=1)
        assert close_errors
        assert holder.is_alive()
        assert dispatcher._state._connection_generation is None
        assert dispatcher._state._sessions == {}
        assert dispatcher._state._computers["thread-1"] is controller
    finally:
        release_lock.set()
        holder.join(timeout=2)
        closer.join(timeout=2)
        bridge_thread = bridge._thread
        if bridge_thread is not None:
            bridge_thread.join(timeout=2)
        bridge.close()

    assert not holder.is_alive()
    assert not closer.is_alive()

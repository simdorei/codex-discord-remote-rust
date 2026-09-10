from __future__ import annotations

import threading
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_dispatch import ActiveProject, LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    OperationErrorResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import (
    ComputerScreenshotOutput,
    ComputerStopOutput,
    ComputerWindowsOutput,
)
from simdorei_mcp_common.operation_requests import (
    ComputerClickRequest,
    ComputerListWindowsRequest,
    ComputerScreenshotRequest,
    ComputerStopRequest,
)
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)

COMPUTER_SESSION_A = "computer-session-a"
COMPUTER_SESSION_B = "computer-session-b"


def _activate(
    dispatcher: LocalProjectDispatcher,
    thread_id: str,
    computer_session_id: str,
) -> None:
    result = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId(f"activate-{thread_id}-{computer_session_id}"),
            thread_id=thread_id,
            computer_session_id=computer_session_id,
            computer_session_generation=1,
        )
    )
    assert isinstance(result, ProjectSessionResult)


def test_emergency_stop_blocks_only_until_project_is_rebound(tmp_path: Path) -> None:
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform)
    )
    expires_at = datetime.now(UTC) + timedelta(minutes=10)
    dispatcher.upsert("thread-a", tmp_path, expires_at)
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_A)

    listed = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-list"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerListWindowsRequest(),
        )
    )
    assert isinstance(listed, ProjectOperationResult)
    assert isinstance(listed.output, ComputerWindowsOutput)

    stopped = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-stop"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerStopRequest(),
        )
    )
    assert isinstance(stopped, ProjectOperationResult)
    assert isinstance(stopped.output, ComputerStopOutput)

    blocked = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-blocked"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerListWindowsRequest(),
        )
    )
    assert isinstance(blocked, OperationErrorResult)
    assert blocked.error_code == "computer_control_stopped"

    dispatcher.upsert("thread-a", tmp_path, expires_at)
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_B)
    rebound = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-rebound"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_B,
            operation=ComputerListWindowsRequest(),
        )
    )
    assert isinstance(rebound, ProjectOperationResult)
    assert isinstance(rebound.output, ComputerWindowsOutput)


def test_screenshot_tokens_are_isolated_between_threads(tmp_path: Path) -> None:
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    expires_at = datetime.now(UTC) + timedelta(minutes=10)
    dispatcher.upsert("thread-a", tmp_path, expires_at)
    dispatcher.upsert("thread-b", tmp_path, expires_at)
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_A)
    _activate(dispatcher, "thread-b", COMPUTER_SESSION_B)
    captured = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-a"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )
    assert isinstance(captured, ProjectOperationResult)
    assert isinstance(captured.output, ComputerScreenshotOutput)

    hijack = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-click-b"),
            thread_id="thread-b",
            computer_session_id=COMPUTER_SESSION_B,
            operation=ComputerClickRequest(
                window_id=42,
                observation_id=captured.output.observation_id,
                x=10,
                y=10,
            ),
        )
    )
    assert isinstance(hijack, OperationErrorResult)
    assert hijack.error_code == "computer_control"
    assert platform.clicks == []


def test_transport_disconnect_revokes_existing_screenshot_tokens(
    tmp_path: Path,
) -> None:
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    expires_at = datetime.now(UTC) + timedelta(minutes=10)
    dispatcher.upsert("thread-a", tmp_path, expires_at)
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_A)
    captured = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-before-disconnect"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )
    assert isinstance(captured, ProjectOperationResult)
    assert isinstance(captured.output, ComputerScreenshotOutput)

    dispatcher.invalidate_computer_sessions()
    stale = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-stale-after-disconnect"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerClickRequest(
                window_id=42,
                observation_id=captured.output.observation_id,
                x=10,
                y=10,
            ),
        )
    )
    still_revoked = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-after-reconnect"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_B)
    fresh = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-after-reselect"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_B,
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )

    assert isinstance(stale, OperationErrorResult)
    assert stale.error_code == "computer_control"
    assert isinstance(still_revoked, OperationErrorResult)
    assert isinstance(fresh, ProjectOperationResult)
    assert isinstance(fresh.output, ComputerScreenshotOutput)


def test_stop_between_state_check_and_controller_creation_blocks_operation(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    _activate(dispatcher, "thread-a", COMPUTER_SESSION_A)
    original = dispatcher._computer_for
    waiting = threading.Event()
    release = threading.Event()

    def delayed_controller(
        thread_id: str,
        project: ActiveProject,
        computer_session_id: str | None,
        connection_generation: int | None,
    ) -> ComputerController | None:
        waiting.set()
        if not release.wait(timeout=2):
            raise AssertionError("controller creation was not released")
        return original(
            thread_id,
            project,
            computer_session_id,
            connection_generation,
        )

    monkeypatch.setattr(dispatcher, "_computer_for", delayed_controller)
    outcome: list[ProjectOperationResult | OperationErrorResult] = []

    def list_windows() -> None:
        result = dispatcher.execute(
            ProjectOperationCommand(
                request_id=RequestId("request-racing-list"),
                thread_id="thread-a",
                computer_session_id=COMPUTER_SESSION_A,
                operation=ComputerListWindowsRequest(),
            )
        )
        assert isinstance(result, (ProjectOperationResult, OperationErrorResult))
        outcome.append(result)

    worker = threading.Thread(target=list_windows)
    worker.start()
    assert waiting.wait(timeout=2)
    stopped = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-racing-stop"),
            thread_id="thread-a",
            computer_session_id=COMPUTER_SESSION_A,
            operation=ComputerStopRequest(),
        )
    )
    release.set()
    worker.join(timeout=2)

    assert isinstance(stopped, ProjectOperationResult)
    assert len(outcome) == 1
    assert isinstance(outcome[0], OperationErrorResult)
    assert outcome[0].error_code == "computer_control_stopped"

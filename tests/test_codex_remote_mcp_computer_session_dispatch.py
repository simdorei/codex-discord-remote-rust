from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from simdorei_mcp_common.messages import (
    OperationErrorResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    ReadFileCommand,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import ComputerScreenshotOutput
from simdorei_mcp_common.operation_requests import (
    ComputerClickRequest,
    ComputerScreenshotRequest,
    FileCreateRequest,
)
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


def _activate(
    dispatcher: LocalProjectDispatcher,
    session_id: str,
    session_generation: int,
) -> None:
    result = dispatcher.execute(
        ProjectSessionCommand(
            request_id=RequestId(f"activate-{session_id}"),
            thread_id="thread-a",
            computer_session_id=session_id,
            computer_session_generation=session_generation,
        )
    )
    assert isinstance(result, ProjectSessionResult)


def test_new_chat_session_revokes_previous_screenshot_token(tmp_path: Path) -> None:
    platform = FakeComputerPlatform(computer_window())
    dispatcher = LocalProjectDispatcher(
        computer_factory=lambda: make_controller(platform),
    )
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    _activate(dispatcher, "computer-session-a", 1)
    captured = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-capture-session-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            operation=ComputerScreenshotRequest(window_id=42),
        )
    )
    assert isinstance(captured, ProjectOperationResult)
    assert isinstance(captured.output, ComputerScreenshotOutput)

    _activate(dispatcher, "computer-session-b", 2)
    stale = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-click-session-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            operation=ComputerClickRequest(
                window_id=42,
                observation_id=captured.output.observation_id,
                x=10,
                y=10,
            ),
        )
    )

    assert isinstance(stale, OperationErrorResult)
    assert stale.error_code == "computer_control"
    assert platform.clicks == []


def test_new_chat_session_rejects_stale_file_mutation(tmp_path: Path) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    _activate(dispatcher, "computer-session-a", 1)
    _activate(dispatcher, "computer-session-b", 2)

    stale = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("request-create-session-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            operation=FileCreateRequest(path="stale.txt", content="stale work"),
        )
    )

    assert isinstance(stale, OperationErrorResult)
    assert stale.error_code == "computer_control"
    assert not (tmp_path / "stale.txt").exists()


def test_new_chat_session_rejects_stale_legacy_file_command(tmp_path: Path) -> None:
    (tmp_path / "notes.txt").write_text("current", encoding="utf-8")
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    _activate(dispatcher, "computer-session-a", 1)
    _activate(dispatcher, "computer-session-b", 2)

    stale = dispatcher.execute(
        ReadFileCommand(
            request_id=RequestId("request-read-session-a"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            path="notes.txt",
            start_line=1,
            max_lines=20,
        )
    )

    assert isinstance(stale, OperationErrorResult)
    assert stale.error_code == "computer_control"

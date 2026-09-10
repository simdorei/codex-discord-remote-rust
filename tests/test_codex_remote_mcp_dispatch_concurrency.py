from __future__ import annotations

import threading
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from codex_remote_mcp_dispatch_commands import (
    BoundProjectCommand,
    execute_bound_project_command,
)
from codex_remote_mcp_files import ProjectFileAccess
from simdorei_mcp_common.messages import (
    BridgeResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ProjectSessionCommand,
    ProjectSessionResult,
    RequestId,
)
from simdorei_mcp_common.operation_requests import (
    ComputerListWindowsRequest,
    FileCreateRequest,
)
from simdorei_mcp_common.request_deadlines import RequestBudget
from tests.remote_mcp_computer_fakes import (
    FakeComputerPlatform,
    computer_window,
    make_controller,
)


def _session_command(
    session_id: str,
    session_generation: int,
) -> ProjectSessionCommand:
    return ProjectSessionCommand(
        request_id=RequestId(f"activate-{session_id}"),
        thread_id="thread-a",
        computer_session_id=session_id,
        computer_session_generation=session_generation,
    )


def test_replacement_waits_for_an_admitted_mutation(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    assert isinstance(
        dispatcher.execute(_session_command("computer-session-a", 1)),
        ProjectSessionResult,
    )

    mutation_started = threading.Event()
    release_mutation = threading.Event()
    replacement_completed = threading.Event()
    completion_order: list[str] = []
    def delayed_execute(
        command: BoundProjectCommand,
        access: ProjectFileAccess,
        computer: ComputerController | None,
        *,
        budget: RequestBudget,
    ) -> BridgeResult:
        is_delayed_create = (
            isinstance(command, ProjectOperationCommand)
            and isinstance(command.operation, FileCreateRequest)
        )
        if is_delayed_create:
            mutation_started.set()
            assert release_mutation.wait(timeout=5)
        result = execute_bound_project_command(
            command,
            access,
            computer,
            budget=budget,
        )
        if is_delayed_create:
            completion_order.append("mutation")
        return result

    monkeypatch.setattr(
        "codex_remote_mcp_dispatch.execute_bound_project_command",
        delayed_execute,
    )

    mutation_results: list[BridgeResult] = []
    replacement_results: list[BridgeResult] = []

    def mutate() -> None:
        mutation_results.append(
            dispatcher.execute(
                ProjectOperationCommand(
                    request_id=RequestId("create-before-replacement"),
                    thread_id="thread-a",
                    computer_session_id="computer-session-a",
                    operation=FileCreateRequest(
                        path="before-replacement.txt",
                        content="completed before replacement",
                    ),
                )
            )
        )

    def replace() -> None:
        replacement_results.append(
            dispatcher.execute(_session_command("computer-session-b", 2))
        )
        completion_order.append("replacement")
        replacement_completed.set()

    mutation_thread = threading.Thread(target=mutate)
    replacement_thread = threading.Thread(target=replace)
    mutation_thread.start()
    assert mutation_started.wait(timeout=5)
    replacement_thread.start()

    assert not replacement_completed.wait(timeout=0.2)
    release_mutation.set()
    mutation_thread.join(timeout=5)
    replacement_thread.join(timeout=5)

    assert not mutation_thread.is_alive()
    assert not replacement_thread.is_alive()
    assert isinstance(mutation_results[0], ProjectOperationResult)
    assert isinstance(replacement_results[0], ProjectSessionResult)
    assert completion_order == ["mutation", "replacement"]
    assert (tmp_path / "before-replacement.txt").read_text(
        encoding="utf-8"
    ) == "completed before replacement"


def test_replacement_reports_app_cleanup_failure_and_allows_a_retry(
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
    assert isinstance(
        dispatcher.execute(_session_command("computer-session-a", 1)),
        ProjectSessionResult,
    )
    listed = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("create-controller"),
            thread_id="thread-a",
            computer_session_id="computer-session-a",
            operation=ComputerListWindowsRequest(),
        )
    )
    assert isinstance(listed, ProjectOperationResult)
    attempts: list[bool] = []

    def flaky_stop(_: FakeComputerPlatform) -> None:
        attempts.append(True)
        if len(attempts) == 1:
            raise ComputerControlError("temporary cleanup failure")

    monkeypatch.setattr(FakeComputerPlatform, "stop", flaky_stop)

    failed = dispatcher.execute(_session_command("computer-session-b", 2))
    retried = dispatcher.execute(_session_command("computer-session-b", 2))

    assert failed.type == "operation_error"
    assert failed.error_code == "computer_control"
    assert isinstance(retried, ProjectSessionResult)
    assert attempts == [True, True]


def test_older_session_generation_cannot_replace_a_newer_session(
    tmp_path: Path,
) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )

    newer = dispatcher.execute(_session_command("computer-session-b", 2))
    older = dispatcher.execute(_session_command("computer-session-a", 1))
    current = dispatcher.execute(
        ProjectOperationCommand(
            request_id=RequestId("create-with-current-session"),
            thread_id="thread-a",
            computer_session_id="computer-session-b",
            operation=FileCreateRequest(path="current.txt", content="current"),
        )
    )

    assert isinstance(newer, ProjectSessionResult)
    assert older.type == "operation_error"
    assert older.error_code == "computer_control"
    assert isinstance(current, ProjectOperationResult)
    assert (tmp_path / "current.txt").read_text(encoding="utf-8") == "current"


def test_same_session_generation_rejects_a_different_session_id(
    tmp_path: Path,
) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )

    first = dispatcher.execute(_session_command("computer-session-a", 1))
    retry = dispatcher.execute(_session_command("computer-session-a", 1))
    conflict = dispatcher.execute(_session_command("computer-session-b", 1))

    assert isinstance(first, ProjectSessionResult)
    assert isinstance(retry, ProjectSessionResult)
    assert conflict.type == "operation_error"
    assert conflict.error_code == "computer_control"


def test_old_connection_cannot_activate_after_a_reconnect(tmp_path: Path) -> None:
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        tmp_path,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    dispatcher.begin_connection(1)
    dispatcher.begin_connection(2)

    newer = dispatcher.execute(
        _session_command("computer-session-new", 1),
        connection_generation=2,
    )
    older_connection = dispatcher.execute(
        _session_command("computer-session-old", 99),
        connection_generation=1,
    )

    assert isinstance(newer, ProjectSessionResult)
    assert older_connection.type == "operation_error"
    assert older_connection.error_code == "computer_control"

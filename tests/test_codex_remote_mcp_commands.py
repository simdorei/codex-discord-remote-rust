# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

import json
import shutil
from collections.abc import Iterator
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import assert_never
from uuid import uuid4

import pytest
from pydantic import ValidationError

from codex_remote_mcp_dispatch import LocalProjectDispatcher
from codex_remote_mcp_commands import list_commands
from simdorei_mcp_common.messages import (
    ListFilesResult,
    OperationErrorResult,
    ProjectInfoResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    ReadFileResult,
    RequestId,
    WriteFileResult,
)
from simdorei_mcp_common.operation_outputs import (
    CommandListOutput,
    CommandRunOutput,
)
from simdorei_mcp_common.operation_requests import (
    CommandListRequest,
    CommandRunRequest,
)
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)


def test_command_list_discovers_package_script(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    _write_package(root)

    # When
    result = dispatcher.execute(_command("list", CommandListRequest()))

    # Then
    match result:
        case ProjectOperationResult(output=CommandListOutput(commands=commands)):
            assert commands[0].command_id == "npm:test"
            assert commands[0].risk_tier == "verify"
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | OperationErrorResult()
        ):
            raise AssertionError(f"unexpected result: {result.type}")
        case unreachable:
            assert_never(unreachable)


def test_command_list_obeys_request_budget(tmp_path: Path) -> None:
    root = tmp_path / "project"
    root.mkdir()
    budget = RequestBudget(_deadline_monotonic=0.0, _clock=lambda: 1.0)

    with pytest.raises(RequestDeadlineExpired):
        _ = list_commands(root, budget=budget)


def test_command_list_rejects_oversized_package_manifest(tmp_path: Path) -> None:
    root, dispatcher = _bound_project(tmp_path)
    package = root / "package.json"
    with package.open("wb") as stream:
        _ = stream.write(b"{")
        _ = stream.seek(1_048_576)
        _ = stream.write(b"}")

    result = dispatcher.execute(_command("large-manifest", CommandListRequest()))

    assert isinstance(result, OperationErrorResult)
    assert "file exceeds" in result.message


def test_command_run_executes_discovered_verification_script(
    command_project_root: Path,
) -> None:
    # Given
    root, dispatcher = _bound_project(command_project_root)
    _write_package(root)

    # When
    result = dispatcher.execute(
        _command(
            "run",
            CommandRunRequest(command_id="npm:test"),
        )
    )

    # Then
    match result:
        case ProjectOperationResult(
            output=CommandRunOutput(exit_code=code, stdout=stdout)
        ):
            assert code == 0
            assert "verified" in stdout
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | OperationErrorResult()
        ):
            raise AssertionError(f"unexpected result: {result.type}")
        case unreachable:
            assert_never(unreachable)


def test_command_run_bounds_noisy_output_without_changing_success(
    command_project_root: Path,
) -> None:
    root, dispatcher = _bound_project(command_project_root)
    tests = root / "tests"
    tests.mkdir()
    _ = (tests / "verified.test.mjs").write_text(
        "import test from 'node:test';\n"
        + "test('noisy', () => console.log('x'.repeat(20000)));\n",
        encoding="utf-8",
    )
    _ = (root / "package.json").write_text(
        json.dumps(
            {
                "scripts": {
                    "test": (
                        "node --test --test-isolation=none "
                        "tests/verified.test.mjs"
                    )
                }
            }
        ),
        encoding="utf-8",
    )

    result = dispatcher.execute(
        _command("noisy", CommandRunRequest(command_id="npm:test"))
    )

    match result:
        case ProjectOperationResult(
            output=CommandRunOutput(
                exit_code=0,
                stdout=stdout,
                truncated=True,
            )
        ):
            assert "...[output truncated]..." in stdout
            assert len(stdout.encode("utf-8")) <= 12_000
        case _:
            raise AssertionError(f"unexpected result: {result}")


def test_command_run_rejects_caller_supplied_arguments() -> None:
    # Given / When / Then
    with pytest.raises(ValidationError, match="Extra inputs are not permitted"):
        _ = CommandRunRequest.model_validate(
            {
                "command_id": "npm:test",
                "args": ("--import=data:text/javascript,console.log('unsafe')",),
            }
        )


def test_command_run_rejects_manifest_shell_body(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    outside = tmp_path / "escaped.txt"
    _ = (root / "package.json").write_text(
        json.dumps(
            {
                "scripts": {
                    "test": (
                        "node -e \"require('fs').writeFileSync("
                        f"'{outside.as_posix()}','escaped')\""
                    )
                }
            }
        ),
        encoding="utf-8",
    )

    # When
    result = dispatcher.execute(
        _command("unsafe", CommandRunRequest(command_id="npm:test"))
    )

    # Then
    assert isinstance(result, OperationErrorResult)
    assert "not remotely executable" in result.message
    assert not outside.exists()


def _bound_project(tmp_path: Path) -> tuple[Path, LocalProjectDispatcher]:
    root = tmp_path / "project"
    root.mkdir()
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        root,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    activate_test_session(dispatcher)
    return root, dispatcher


@pytest.fixture
def command_project_root() -> Iterator[Path]:
    repo_root = Path(__file__).resolve().parents[1]
    qa_root = repo_root / f".remote-command-qa-{uuid4().hex}"
    qa_root.mkdir()
    try:
        yield qa_root
    finally:
        shutil.rmtree(qa_root)


def _write_package(root: Path) -> None:
    tests = root / "tests"
    tests.mkdir()
    _ = (tests / "verified.test.mjs").write_text(
        "import test from 'node:test';\n"
        + "test('verified', () => console.log('verified'));\n",
        encoding="utf-8",
    )
    _ = (root / "package.json").write_text(
        json.dumps(
            {
                "scripts": {
                    "test": (
                        "node --test --test-isolation=none "
                        "tests/verified.test.mjs"
                    ),
                }
            }
        ),
        encoding="utf-8",
    )


def _command(
    suffix: str,
    operation: CommandListRequest | CommandRunRequest,
) -> ProjectOperationCommand:
    return ProjectOperationCommand(
        request_id=RequestId(f"request-{suffix}"),
        thread_id="thread-a",
        computer_session_id=TEST_PROJECT_SESSION_ID,
        operation=operation,
    )

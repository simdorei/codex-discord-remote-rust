from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import assert_never

import pytest

from codex_remote_mcp_checkpoints import CheckpointDraft
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from codex_remote_mcp_files import ProjectFileAccess
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
from simdorei_mcp_common.operation_outputs import FileApplyPatchOutput
from simdorei_mcp_common.operation_requests import FileApplyPatchRequest, FileChange
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)


def test_patch_updates_file_with_matching_hash(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    target = root / "notes.txt"
    target.write_text("first\nsecond\n", encoding="utf-8")
    current_hash = _hash(root, "notes.txt")

    # When
    result = dispatcher.execute(
        _command(
            "update",
            (
                FileChange(
                    action="update",
                    path="notes.txt",
                    content="first\nchanged\n",
                    expected_sha256=current_hash,
                ),
            ),
        )
    )

    # Then
    _assert_action(result, "update")
    assert target.read_text(encoding="utf-8") == "first\nchanged\n"


def test_patch_deletes_file_with_matching_hash(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    target = root / "gone.txt"
    target.write_text("delete me", encoding="utf-8")
    current_hash = _hash(root, "gone.txt")

    # When
    result = dispatcher.execute(
        _command(
            "delete",
            (
                FileChange(
                    action="delete",
                    path="gone.txt",
                    expected_sha256=current_hash,
                ),
            ),
        )
    )

    # Then
    _assert_action(result, "delete")
    assert not target.exists()


def test_patch_moves_file_with_matching_hash(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    source = root / "old.txt"
    source.write_text("move me\n", encoding="utf-8")
    current_hash = _hash(root, "old.txt")

    # When
    result = dispatcher.execute(
        _command(
            "move",
            (
                FileChange(
                    action="move",
                    path="old.txt",
                    destination="new.txt",
                    expected_sha256=current_hash,
                ),
            ),
        )
    )

    # Then
    _assert_action(result, "move")
    assert not source.exists()
    assert (root / "new.txt").read_text(encoding="utf-8") == "move me\n"


def test_patch_rolls_back_when_checkpoint_persist_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)

    def fail_checkpoint(_draft: CheckpointDraft) -> str:
        raise OSError("disk failure")

    monkeypatch.setattr(
        "codex_remote_mcp_patch.finish_checkpoint",
        fail_checkpoint,
    )

    # When
    with pytest.raises(OSError, match="disk failure"):
        dispatcher.execute(
            _command(
                "rollback",
                (
                    FileChange(action="create", path="first.txt", content="first\n"),
                    FileChange(action="create", path="second.txt", content="second\n"),
                ),
            )
        )

    # Then
    assert not (root / "first.txt").exists()
    assert not (root / "second.txt").exists()


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


def _hash(root: Path, path: str) -> str:
    return ProjectFileAccess(root).read_file(
        path,
        start_line=1,
        max_lines=20,
    ).sha256


def _command(
    suffix: str,
    changes: tuple[FileChange, ...],
) -> ProjectOperationCommand:
    return ProjectOperationCommand(
        request_id=RequestId(f"request-{suffix}"),
        thread_id="thread-a",
        computer_session_id=TEST_PROJECT_SESSION_ID,
        operation=FileApplyPatchRequest(changes=changes),
    )


def _assert_action(
    result: (
        ProjectInfoResult
        | ListFilesResult
        | ReadFileResult
        | WriteFileResult
        | ProjectOperationResult
        | OperationErrorResult
    ),
    expected: str,
) -> None:
    match result:
        case ProjectOperationResult(output=FileApplyPatchOutput(applied=applied)):
            assert applied[0].action == expected
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

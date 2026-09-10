from __future__ import annotations

# pyright: reportUnknownArgumentType=false, reportUnknownLambdaType=false
# pyright: reportUnusedCallResult=false
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

import codex_remote_mcp_checkpoints

from codex_remote_mcp_checkpoint_transaction import CheckpointDraft, CheckpointTarget
from codex_remote_mcp_checkpoints import (
    begin_checkpoint,
    finish_checkpoint,
    restore_checkpoint,
)
from codex_remote_mcp_dispatch import LocalProjectDispatcher
from codex_remote_mcp_files import FileConflictError, ProjectFileAccess
from simdorei_mcp_common.messages import (
    OperationErrorResult,
    ProjectOperationCommand,
    ProjectOperationResult,
    RequestId,
)
from simdorei_mcp_common.operation_outputs import (
    CheckpointListOutput,
    CheckpointRestoreOutput,
    FileCreateOutput,
    ProjectStatusOutput,
)
from simdorei_mcp_common.operation_requests import (
    CheckpointListRequest,
    CheckpointRestoreRequest,
    FileCreateRequest,
    ProjectStatusRequest,
)
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)


def test_checkpoint_list_reports_file_create_checkpoint(tmp_path: Path) -> None:
    # Given
    _, dispatcher = _bound_project(tmp_path)
    created = dispatcher.execute(
        _command("create", FileCreateRequest(path="note.txt", content="new"))
    )
    assert isinstance(created, ProjectOperationResult)

    # When
    result = dispatcher.execute(_command("list", CheckpointListRequest()))

    # Then
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, CheckpointListOutput)
    assert len(result.output.checkpoints) == 1


def test_checkpoint_list_rejects_directory_beyond_scan_limit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root, dispatcher = _bound_project(tmp_path)
    directory = root / ".codex-remote-mcp" / "checkpoints"
    directory.mkdir(parents=True)
    for index in range(3):
        (directory / f"noise-{index}").write_text("x", encoding="utf-8")
    monkeypatch.setattr(
        codex_remote_mcp_checkpoints,
        "MAX_CHECKPOINT_SCAN_CANDIDATES",
        2,
    )

    result = dispatcher.execute(_command("bounded-list", CheckpointListRequest()))

    assert isinstance(result, OperationErrorResult)
    assert "scanned too many candidates" in result.message


def test_checkpoint_retention_prunes_old_records(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root, dispatcher = _bound_project(tmp_path)
    monkeypatch.setattr(codex_remote_mcp_checkpoints, "MAX_CHECKPOINT_RETAINED", 2)

    for index in range(3):
        result = dispatcher.execute(
            _command(
                f"retained-{index}",
                FileCreateRequest(path=f"note-{index}.txt", content=str(index)),
            )
        )
        assert isinstance(result, ProjectOperationResult)

    records = tuple(
        (root / ".codex-remote-mcp" / "checkpoints").glob("cp_*.json")
    )
    assert len(records) == 2


def test_checkpoint_restore_removes_created_file(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    created = dispatcher.execute(
        _command("create", FileCreateRequest(path="note.txt", content="new"))
    )
    assert isinstance(created, ProjectOperationResult)
    assert isinstance(created.output, FileCreateOutput)
    checkpoint_id = created.output.checkpoint_id

    # When
    result = dispatcher.execute(
        _command(
            "restore",
            CheckpointRestoreRequest(checkpoint_id=checkpoint_id),
        )
    )

    # Then
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, CheckpointRestoreOutput)
    assert not (root / "note.txt").exists()


def test_project_status_combines_git_rules_and_commands(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    (root / "AGENTS.md").write_text("rules", encoding="utf-8")
    (root / "package.json").write_text(
        '{"scripts":{"test":"node --test"}}',
        encoding="utf-8",
    )
    _git(root, "init")

    # When
    result = dispatcher.execute(_command("status", ProjectStatusRequest()))

    # Then
    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, ProjectStatusOutput)
    assert result.output.rule_files == ("AGENTS.md",)
    assert result.output.command_ids == ("npm:test",)


def test_checkpoint_restore_rejects_later_file_change(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _bound_project(tmp_path)
    created = dispatcher.execute(
        _command("create", FileCreateRequest(path="note.txt", content="new"))
    )
    assert isinstance(created, ProjectOperationResult)
    assert isinstance(created.output, FileCreateOutput)
    (root / "note.txt").write_text("later", encoding="utf-8")

    # When / Then
    with pytest.raises(FileConflictError, match="changed after checkpoint"):
        restore_checkpoint(
            root,
            created.output.checkpoint_id,
            budget=_live_budget(),
        )
    assert (root / "note.txt").read_text(encoding="utf-8") == "later"


def test_file_create_rolls_back_when_checkpoint_persist_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root, dispatcher = _bound_project(tmp_path)
    monkeypatch.setattr(
        "codex_remote_mcp_operations.finish_checkpoint",
        lambda _draft: (_ for _ in ()).throw(OSError("disk failure")),
    )

    with pytest.raises(OSError, match="disk failure"):
        dispatcher.execute(
            _command(
                "rollback-create",
                FileCreateRequest(path="note.txt", content="new"),
            )
        )

    assert not (root / "note.txt").exists()


def test_file_create_does_not_delete_a_concurrent_writer_during_rollback(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root, dispatcher = _bound_project(tmp_path)

    def replace_then_fail(_draft: CheckpointDraft) -> str:
        (root / "note.txt").write_text("concurrent-writer", encoding="utf-8")
        raise OSError("disk failure")

    monkeypatch.setattr(
        "codex_remote_mcp_operations.finish_checkpoint",
        replace_then_fail,
    )

    result = dispatcher.execute(
        _command(
            "concurrent-rollback",
            FileCreateRequest(path="note.txt", content="new"),
        )
    )

    assert isinstance(result, OperationErrorResult)
    assert "changed during rollback" in result.message
    assert (root / "note.txt").read_text(encoding="utf-8") == "concurrent-writer"


def test_checkpoint_restore_rolls_back_every_file_when_a_later_write_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root = tmp_path / "project"
    root.mkdir()
    first = root / "first.txt"
    second = root / "second.txt"
    first.write_text("first-before", encoding="utf-8")
    second.write_text("second-before", encoding="utf-8")
    targets = tuple(CheckpointTarget(path.name, path) for path in (first, second))
    draft = begin_checkpoint(root, "two-file mutation", targets)
    access = ProjectFileAccess(root)
    for path, content in (
        ("first.txt", b"first-after"),
        ("second.txt", b"second-after"),
    ):
        current = access.read_file(path, start_line=1, max_lines=10)
        _ = access.write_bytes(path, content, expected_sha256=current.sha256)
    checkpoint_id = finish_checkpoint(draft)
    original_write = ProjectFileAccess.write_bytes
    failed = False

    def fail_second_once(
        self: ProjectFileAccess,
        value: str,
        content: bytes,
        *,
        expected_sha256: str | None,
    ) -> bool:
        nonlocal failed
        if value == "second.txt" and not failed:
            failed = True
            raise OSError("simulated second-file failure")
        return original_write(
            self,
            value,
            content,
            expected_sha256=expected_sha256,
        )

    monkeypatch.setattr(ProjectFileAccess, "write_bytes", fail_second_once)

    with pytest.raises(OSError, match="second-file failure"):
        restore_checkpoint(root, checkpoint_id, budget=_live_budget())

    assert first.read_text(encoding="utf-8") == "first-after"
    assert second.read_text(encoding="utf-8") == "second-after"


def test_checkpoint_restore_budget_expiry_rolls_back_partial_restore(
    tmp_path: Path,
) -> None:
    root = tmp_path / "project"
    root.mkdir()
    first = root / "first.txt"
    second = root / "second.txt"
    first.write_text("first-before", encoding="utf-8")
    second.write_text("second-before", encoding="utf-8")
    targets = tuple(CheckpointTarget(path.name, path) for path in (first, second))
    draft = begin_checkpoint(root, "two-file mutation", targets)
    first.write_text("first-after", encoding="utf-8")
    second.write_text("second-after", encoding="utf-8")
    checkpoint_id = finish_checkpoint(draft)

    clock_checks = 0

    def expiring_clock() -> float:
        nonlocal clock_checks
        clock_checks += 1
        return 0.0 if clock_checks < 4 else 2.0

    budget = RequestBudget(
        _deadline_monotonic=1.0,
        _clock=expiring_clock,
    )

    before_checkpoints = tuple(
        (root / ".codex-remote-mcp" / "checkpoints").glob("cp_*.json")
    )
    with pytest.raises(RequestDeadlineExpired, match="expired before execution"):
        _ = restore_checkpoint(
            root,
            checkpoint_id,
            budget=budget,
        )

    assert first.read_text(encoding="utf-8") == "first-after"
    assert second.read_text(encoding="utf-8") == "second-after"
    after_checkpoints = tuple(
        (root / ".codex-remote-mcp" / "checkpoints").glob("cp_*.json")
    )
    assert after_checkpoints == before_checkpoints


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


def _live_budget() -> RequestBudget:
    return RequestBudget.from_deadline(datetime.now(UTC) + timedelta(minutes=1))


def _command(
    suffix: str,
    operation: (
        CheckpointListRequest
        | CheckpointRestoreRequest
        | FileCreateRequest
        | ProjectStatusRequest
    ),
) -> ProjectOperationCommand:
    return ProjectOperationCommand.model_validate(
        {
            "request_id": RequestId(f"request-{suffix}"),
            "thread_id": "thread-a",
            "computer_session_id": TEST_PROJECT_SESSION_ID,
            "operation": operation,
        }
    )


def _git(root: Path, *arguments: str) -> None:
    import subprocess

    subprocess.run(
        ("git", *arguments),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )

from __future__ import annotations

import subprocess
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import assert_never

import pytest
from pydantic import ValidationError

from codex_remote_mcp_dispatch import LocalProjectDispatcher
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
    GitCommitOutput,
    GitPushOutput,
    RepoDiffOutput,
    RepoStatusOutput,
)
from simdorei_mcp_common.operation_requests import (
    GitCommitRequest,
    GitPushRequest,
    RepoDiffRequest,
    RepoStatusRequest,
)
from tests.remote_mcp_dispatch_support import (
    TEST_PROJECT_SESSION_ID,
    activate_test_session,
)


def test_repo_status_and_diff_report_working_change(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _git_project(tmp_path)
    target = root / "notes.txt"
    target.write_text("changed\n", encoding="utf-8")

    # When
    status = dispatcher.execute(_command("status", RepoStatusRequest()))
    diff = dispatcher.execute(_command("diff", RepoDiffRequest()))

    # Then
    match status:
        case ProjectOperationResult(output=RepoStatusOutput(dirty_files=dirty)):
            assert dirty == ("notes.txt",)
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | OperationErrorResult()
        ):
            raise AssertionError(f"unexpected result: {status.type}")
        case unreachable:
            assert_never(unreachable)
    match diff:
        case ProjectOperationResult(output=RepoDiffOutput(files=files, patch=patch)):
            assert files[0].path == "notes.txt"
            assert "+changed" in patch
        case (
            ProjectInfoResult()
            | ListFilesResult()
            | ReadFileResult()
            | WriteFileResult()
            | ProjectOperationResult()
            | OperationErrorResult()
        ):
            raise AssertionError(f"unexpected result: {diff.type}")
        case unreachable:
            assert_never(unreachable)


def test_repo_diff_returns_bounded_partial_patch_for_large_change(
    tmp_path: Path,
) -> None:
    root, dispatcher = _git_project(tmp_path)
    (root / "notes.txt").write_text("x" * 500_000 + "\n", encoding="utf-8")

    result = dispatcher.execute(_command("large-diff", RepoDiffRequest()))

    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, RepoDiffOutput)
    assert result.output.truncated
    assert len(result.output.patch) == 200_000
    assert result.output.files[0].path == "notes.txt"


def test_git_commit_stages_and_commits_selected_file(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _git_project(tmp_path)
    target = root / "notes.txt"
    target.write_text("changed\n", encoding="utf-8")

    # When
    result = dispatcher.execute(
        _command(
            "commit",
            GitCommitRequest(
                message="test: update notes",
                paths=("notes.txt",),
            ),
        )
    )

    # Then
    match result:
        case ProjectOperationResult(output=GitCommitOutput(commit=commit)):
            assert len(commit) >= 7
            assert _git(root, "status", "--porcelain").stdout == ""
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


def test_repo_diff_includes_untracked_text_file(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _git_project(tmp_path)
    (root / "new.txt").write_text("new line\n", encoding="utf-8")

    # When
    result = dispatcher.execute(_command("untracked", RepoDiffRequest()))

    # Then
    match result:
        case ProjectOperationResult(output=RepoDiffOutput(files=files, patch=patch)):
            assert tuple(file.path for file in files) == ("new.txt",)
            assert "+new line" in patch
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


def test_repo_diff_redacts_secret_values_and_hides_secret_paths(
    tmp_path: Path,
) -> None:
    root, dispatcher = _git_project(tmp_path)
    (root / "settings.py").write_text(
        'api_key = "AbCdEfGh12345678"\n',
        encoding="utf-8",
    )
    (root / ".env.local").write_text("TOKEN=AbCdEfGh12345678\n", encoding="utf-8")

    result = dispatcher.execute(_command("redacted", RepoDiffRequest()))

    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, RepoDiffOutput)
    assert "AbCdEfGh12345678" not in result.output.patch
    assert "[REDACTED]" in result.output.patch
    assert ".env.local" not in result.output.patch
    assert tuple(file.path for file in result.output.files) == ("settings.py",)


def test_git_commit_preserves_other_staged_file(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _git_project(tmp_path)
    (root / "selected.txt").write_text("selected\n", encoding="utf-8")
    (root / "existing.txt").write_text("existing\n", encoding="utf-8")
    _git(root, "add", "existing.txt")

    # When
    result = dispatcher.execute(
        _command(
            "isolated",
            GitCommitRequest(
                message="test: selected only",
                paths=("selected.txt",),
            ),
        )
    )

    # Then
    assert isinstance(result, ProjectOperationResult)
    committed = _git(
        root,
        "diff-tree",
        "--no-commit-id",
        "--name-only",
        "-r",
        "HEAD",
    ).stdout.splitlines()
    assert committed == ["selected.txt"]
    staged = _git(root, "diff", "--cached", "--name-only").stdout.splitlines()
    assert staged == ["existing.txt"]


def test_git_push_rejects_option_like_branch_name() -> None:
    # Given / When / Then
    with pytest.raises(ValidationError):
        GitPushRequest(branch="--all")


def test_git_push_sends_current_branch_to_configured_remote(tmp_path: Path) -> None:
    # Given
    root, dispatcher = _git_project(tmp_path)
    remote = tmp_path / "remote.git"
    remote.mkdir()
    _git(remote, "init", "--bare")
    _git(root, "remote", "add", "origin", str(remote))
    branch = _git(root, "branch", "--show-current").stdout.strip()

    # When
    result = dispatcher.execute(
        _command("push", GitPushRequest(remote="origin", branch=branch))
    )

    # Then
    match result:
        case ProjectOperationResult(output=GitPushOutput(branch=pushed_branch)):
            assert pushed_branch == branch
            remote_head = _git(
                remote,
                "rev-parse",
                f"refs/heads/{branch}",
            ).stdout.strip()
            assert remote_head
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


def _git_project(tmp_path: Path) -> tuple[Path, LocalProjectDispatcher]:
    root = tmp_path / "project"
    root.mkdir()
    _git(root, "init")
    _git(root, "config", "user.name", "Test User")
    _git(root, "config", "user.email", "test@example.com")
    (root / "notes.txt").write_text("before\n", encoding="utf-8")
    _git(root, "add", "notes.txt")
    _git(root, "commit", "-m", "initial")
    dispatcher = LocalProjectDispatcher()
    dispatcher.upsert(
        "thread-a",
        root,
        datetime.now(UTC) + timedelta(minutes=10),
    )
    activate_test_session(dispatcher)
    return root, dispatcher


def _command(
    suffix: str,
    operation: (
        RepoStatusRequest | RepoDiffRequest | GitCommitRequest | GitPushRequest
    ),
) -> ProjectOperationCommand:
    return ProjectOperationCommand(
        request_id=RequestId(f"request-{suffix}"),
        thread_id="thread-a",
        computer_session_id=TEST_PROJECT_SESSION_ID,
        operation=operation,
    )


def _git(root: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ("git", *arguments),
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )

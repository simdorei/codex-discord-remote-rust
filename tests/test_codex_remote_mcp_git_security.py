from __future__ import annotations

from pathlib import Path

import pytest

from simdorei_mcp_common.messages import ProjectOperationResult
from simdorei_mcp_common.operation_outputs import GitCommitOutput
from simdorei_mcp_common.operation_requests import GitCommitRequest
from tests.test_codex_remote_mcp_git import _command, _git_project


def test_git_commit_disables_repository_hooks_and_parent_secrets(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    root, dispatcher = _git_project(tmp_path)
    marker = tmp_path / "hook-secret.txt"
    hook = root / ".git/hooks/pre-commit"
    hook.write_text(
        "#!/bin/sh\n"
        f"printf '%s' \"$PROBE_SECRET\" > '{marker.as_posix()}'\n",
        encoding="utf-8",
    )
    hook.chmod(0o755)
    monkeypatch.setenv("PROBE_SECRET", "credential-value-from-parent")
    (root / "notes.txt").write_text("safe commit\n", encoding="utf-8")

    result = dispatcher.execute(
        _command(
            "hook-safe",
            GitCommitRequest(
                message="test: hooks disabled",
                paths=("notes.txt",),
            ),
        )
    )

    assert isinstance(result, ProjectOperationResult)
    assert isinstance(result.output, GitCommitOutput)
    assert not marker.exists()

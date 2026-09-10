from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

import codex_remote_mcp_git as project_git
import codex_remote_mcp_git_diff as git_diff
from codex_remote_mcp_files import ProjectFileLimitError
from codex_remote_mcp_subprocess import RemoteProcessResult
from simdorei_mcp_common.operation_requests import GitPushRequest
from simdorei_mcp_common.request_deadlines import RequestBudget, RequestDeadlineExpired


class FakeClock:
    def __init__(self) -> None:
        self.current: float = 0.0

    def __call__(self) -> float:
        return self.current

    def advance(self, seconds: float) -> None:
        self.current += seconds


def _budget_with_clock(remaining_seconds: float) -> tuple[RequestBudget, FakeClock]:
    clock = FakeClock()
    return (
        RequestBudget(
            _deadline_monotonic=clock.current + remaining_seconds,
            _clock=clock,
        ),
        clock,
    )


def test_try_git_caps_subprocess_timeout_to_remaining_request_budget(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, _clock = _budget_with_clock(7.5)
    observed_timeouts: list[float] = []

    def fake_run(*args, **kwargs) -> RemoteProcessResult:
        observed_timeouts.append(float(kwargs["timeout_seconds"]))
        return RemoteProcessResult(tuple(args[0]), 0, b"", b"", False, False)

    monkeypatch.setattr(project_git, "run_owned_bounded_process", fake_run)

    _ = project_git._try_git(
        tmp_path,
        ("status", "--porcelain=v1"),
        timeout_seconds=120,
        budget=budget,
    )

    assert observed_timeouts == [7.5]


def test_git_push_preflight_consumption_reduces_push_timeout(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, clock = _budget_with_clock(315)
    effective_timeouts: list[float] = []

    def fake_run_git(
        root: Path,
        arguments: tuple[str, ...],
        *,
        timeout_seconds: float = 120,
        allow_credentials: bool = False,
        budget: RequestBudget,
    ) -> subprocess.CompletedProcess[str]:
        _ = root, allow_credentials
        effective_timeouts.append(budget.remaining(timeout_seconds))
        if arguments == ("remote",):
            clock.advance(20)
            stdout = "origin\n"
        else:
            stdout = "pushed\n"
        return subprocess.CompletedProcess(arguments, 0, stdout, "")

    monkeypatch.setattr(project_git, "_run_git", fake_run_git)

    _ = project_git.git_push(
        tmp_path,
        GitPushRequest(remote="origin", branch="main"),
        budget=budget,
    )

    assert effective_timeouts == [120, 295]


def test_repo_status_stops_before_starting_a_subprocess_after_budget_expires(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, clock = _budget_with_clock(1)
    commands: list[tuple[str, ...]] = []

    def fake_run_git(
        root: Path,
        arguments: tuple[str, ...],
        *,
        timeout_seconds: float = 120,
        allow_credentials: bool = False,
        budget: RequestBudget,
    ) -> subprocess.CompletedProcess[str]:
        _ = root, allow_credentials
        _ = budget.remaining(timeout_seconds)
        commands.append(arguments)
        clock.advance(2)
        return subprocess.CompletedProcess(arguments, 0, "main\n", "")

    monkeypatch.setattr(project_git, "_run_git", fake_run_git)

    with pytest.raises(RequestDeadlineExpired, match="expired before execution"):
        _ = project_git.repo_status(tmp_path, budget=budget)

    assert commands == [("branch", "--show-current")]


def test_repo_diff_stops_untracked_processing_after_budget_expires(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, clock = _budget_with_clock(1)
    calls: list[tuple[str, ...]] = []

    def fake_run_git(
        root: Path,
        arguments: tuple[str, ...],
        **_kwargs: object,
    ) -> subprocess.CompletedProcess[str]:
        _ = root
        calls.append(arguments)
        if arguments[:2] == ("ls-files", "--others"):
            clock.advance(2)
            return subprocess.CompletedProcess(arguments, 0, "a.txt\0", "")
        return subprocess.CompletedProcess(arguments, 0, "", "")

    monkeypatch.setattr(project_git, "_run_git", fake_run_git)

    with pytest.raises(RequestDeadlineExpired):
        _ = project_git.repo_diff(tmp_path, budget=budget)

    assert calls[-1][:2] == ("ls-files", "--others")


def test_repo_diff_caps_untracked_file_candidates(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, _clock = _budget_with_clock(60)
    (tmp_path / "a.txt").write_text("a", encoding="utf-8")
    (tmp_path / "b.txt").write_text("b", encoding="utf-8")
    monkeypatch.setattr(git_diff, "MAX_UNTRACKED_FILES", 1)

    def fake_run(
        root: Path,
        arguments: tuple[str, ...],
        *,
        timeout_seconds: float = 120,
    ) -> subprocess.CompletedProcess[str]:
        _ = root, timeout_seconds
        output = (
            "a.txt\0b.txt\0"
            if arguments[:2] == ("ls-files", "--others")
            else ""
        )
        return subprocess.CompletedProcess(arguments, 0, output, "")

    with pytest.raises(ProjectFileLimitError, match="untracked file count"):
        _ = git_diff.build_repo_diff(
            tmp_path,
            fake_run,
            budget=budget,
        )


def test_credential_helper_local_timeout_becomes_project_git_error(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, _clock = _budget_with_clock(315)
    calls = 0

    def timeout_run(*args, **kwargs):
        nonlocal calls
        calls += 1
        raise subprocess.TimeoutExpired(args[0], kwargs["timeout_seconds"])

    monkeypatch.setattr(project_git, "run_owned_bounded_process", timeout_run)

    with pytest.raises(project_git.ProjectGitError, match="credential helper"):
        _ = project_git._try_git(
            tmp_path,
            ("push", "origin", "main"),
            timeout_seconds=300,
            allow_credentials=True,
            budget=budget,
        )

    assert calls == 1


def test_credential_helper_budget_timeout_stays_request_expired(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, clock = _budget_with_clock(0.01)
    calls = 0

    def timeout_run(*args, **kwargs):
        nonlocal calls
        calls += 1
        clock.advance(1)
        raise subprocess.TimeoutExpired(args[0], kwargs["timeout_seconds"])

    monkeypatch.setattr(project_git, "run_owned_bounded_process", timeout_run)

    with pytest.raises(RequestDeadlineExpired, match="expired before execution"):
        _ = project_git._try_git(
            tmp_path,
            ("push", "origin", "main"),
            timeout_seconds=300,
            allow_credentials=True,
            budget=budget,
        )

    assert calls == 1


def test_missing_credential_helper_git_becomes_project_git_error(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    budget, _clock = _budget_with_clock(315)
    calls = 0

    def missing_run(*_args, **_kwargs):
        nonlocal calls
        calls += 1
        raise FileNotFoundError("git missing")

    monkeypatch.setattr(project_git, "run_owned_bounded_process", missing_run)

    with pytest.raises(project_git.ProjectGitError, match="not found"):
        _ = project_git._try_git(
            tmp_path,
            ("push", "origin", "main"),
            timeout_seconds=300,
            allow_credentials=True,
            budget=budget,
        )

    assert calls == 1

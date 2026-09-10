from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
from functools import partial
from pathlib import Path
from typing import Final

from codex_remote_mcp_files import ProjectFileAccess, ProjectFileError
from codex_remote_mcp_git_diff import build_repo_diff
from codex_remote_mcp_redaction import redact
from codex_remote_mcp_subprocess import (
    RemoteProcessCancelled,
    run_owned_bounded_process,
)
from simdorei_mcp_common.operation_outputs import (
    GitCommitOutput,
    GitPushOutput,
    RepoDiffOutput,
    RepoStatusOutput,
)
from simdorei_mcp_common.operation_requests import GitCommitRequest, GitPushRequest
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)

MAX_GIT_OUTPUT: Final = 200_000
MAX_GIT_CAPTURE_BYTES: Final = 400_000
GIT_TIMEOUT_SECONDS: Final = 120
GIT_ENVIRONMENT_KEYS: Final = (
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemRoot",
    "PATHEXT",
    "LOCALAPPDATA",
    "APPDATA",
    "USERPROFILE",
    "SSH_AUTH_SOCK",
)
SAFE_CREDENTIAL_HELPER: Final = re.compile(
    r"^[A-Za-z0-9._/+:-]+(?:\s+--?[A-Za-z0-9._/=:+-]+)*$"
)


class ProjectGitError(ProjectFileError):
    """Raised when a confined Git operation fails."""


class GitCompletedProcess(subprocess.CompletedProcess[str]):
    """Git result with explicit bounded-capture metadata."""

    truncated: bool

    def __init__(
        self,
        args: tuple[str, ...],
        returncode: int,
        stdout: str,
        stderr: str,
        *,
        truncated: bool,
    ) -> None:
        super().__init__(args, returncode, stdout, stderr)
        self.truncated = truncated


def repo_status(root: Path, *, budget: RequestBudget) -> RepoStatusOutput:
    """Read local repository state without contacting a remote."""
    branch = _run_git(root, ("branch", "--show-current"), budget=budget).stdout.strip()
    porcelain = _run_git(root, ("status", "--porcelain=v1"), budget=budget).stdout
    dirty, staged = _parse_status(porcelain)
    dirty = _visible_paths(root, dirty)
    staged = _visible_paths(root, staged)
    remotes = tuple(
        line.strip()
        for line in _run_git(root, ("remote",), budget=budget).stdout.splitlines()
        if line.strip()
    )
    upstream_result = _try_git(
        root,
        ("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"),
        budget=budget,
    )
    upstream = (
        upstream_result.stdout.strip() if upstream_result.returncode == 0 else None
    )
    ahead, behind = _ahead_behind(root, upstream, budget=budget)
    return RepoStatusOutput(
        branch=branch,
        dirty_files=dirty,
        staged_files=staged,
        remotes=remotes,
        upstream=upstream,
        ahead=ahead,
        behind=behind,
    )


def repo_diff(root: Path, *, budget: RequestBudget) -> RepoDiffOutput:
    """Return a bounded HEAD-relative patch and numeric summary."""
    return build_repo_diff(
        root,
        partial(_run_git, budget=budget),
        partial(_run_git, budget=budget, allow_truncated_output=True),
        budget=budget,
    )


def git_commit(
    root: Path,
    request: GitCommitRequest,
    *,
    budget: RequestBudget,
) -> GitCommitOutput:
    """Stage validated paths and create one local commit."""
    access = ProjectFileAccess(root)
    paths = request.paths
    if not paths:
        raise ProjectGitError(
            "<git>",
            "explicit paths are required to avoid committing unrelated work",
        )
    for path in paths:
        _ = access.resolve_path(path, require_file=False)
    untracked = {
        line.strip()
        for line in _run_git(
            root,
            ("ls-files", "--others", "--exclude-standard"),
            budget=budget,
        ).stdout.splitlines()
        if line.strip()
    }
    new_paths = tuple(path for path in paths if path in untracked)
    if new_paths:
        _ = _run_git(
            root,
            ("add", "--intent-to-add", "--", *new_paths),
            budget=budget,
        )
    _ = _run_git(
        root,
        ("commit", "--only", "-m", request.message, "--", *paths),
        budget=budget,
    )
    commit = _run_git(
        root, ("rev-parse", "--short", "HEAD"), budget=budget
    ).stdout.strip()
    branch = _run_git(root, ("branch", "--show-current"), budget=budget).stdout.strip()
    committed = tuple(
        line.strip()
        for line in _run_git(
            root,
            ("diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"),
            budget=budget,
        ).stdout.splitlines()
        if line.strip()
    )
    return GitCommitOutput(
        commit=commit,
        branch=branch,
        staged_files=committed,
    )


def git_push(
    root: Path,
    request: GitPushRequest,
    *,
    budget: RequestBudget,
) -> GitPushOutput:
    """Push the selected local branch to one configured Git remote."""
    remotes = {
        line.strip()
        for line in _run_git(root, ("remote",), budget=budget).stdout.splitlines()
        if line.strip()
    }
    if request.remote not in remotes:
        raise ProjectGitError(request.remote, "Git remote is not configured")
    branch = (
        request.branch
        or _run_git(
            root,
            ("branch", "--show-current"),
            budget=budget,
        ).stdout.strip()
    )
    if not branch:
        raise ProjectGitError("<git>", "detached HEAD cannot be pushed implicitly")
    result = _run_git(
        root,
        ("push", "-u", "--", request.remote, branch),
        timeout_seconds=300,
        allow_credentials=True,
        budget=budget,
    )
    combined = f"{result.stdout}\n{result.stderr}".strip()
    return GitPushOutput(
        remote=request.remote,
        branch=branch,
        output=redact(combined)[:MAX_GIT_OUTPUT],
    )


def _parse_status(status: str) -> tuple[tuple[str, ...], tuple[str, ...]]:
    dirty: list[str] = []
    staged: list[str] = []
    for line in status.splitlines():
        if len(line) < 4:
            continue
        index_state = line[0]
        worktree_state = line[1]
        path = line[3:]
        if " -> " in path:
            path = path.split(" -> ", maxsplit=1)[1]
        if index_state == "?" and worktree_state == "?":
            dirty.append(path)
            continue
        if index_state not in {" ", "?"}:
            staged.append(path)
        if worktree_state not in {" ", "?"}:
            dirty.append(path)
    return tuple(dirty), tuple(staged)


def _ahead_behind(
    root: Path,
    upstream: str | None,
    *,
    budget: RequestBudget,
) -> tuple[int, int]:
    if upstream is None:
        return 0, 0
    result = _try_git(
        root,
        ("rev-list", "--left-right", "--count", "HEAD...@{upstream}"),
        budget=budget,
    )
    if result.returncode != 0:
        return 0, 0
    values = result.stdout.strip().split()
    if len(values) != 2:
        return 0, 0
    return int(values[0]), int(values[1])


def _run_git(
    root: Path,
    arguments: tuple[str, ...],
    *,
    timeout_seconds: float = GIT_TIMEOUT_SECONDS,
    allow_credentials: bool = False,
    allow_truncated_output: bool = False,
    budget: RequestBudget,
) -> GitCompletedProcess:
    result = _try_git(
        root,
        arguments,
        timeout_seconds=timeout_seconds,
        allow_credentials=allow_credentials,
        allow_truncated_output=allow_truncated_output,
        budget=budget,
    )
    if result.returncode != 0:
        message = redact(result.stderr.strip() or result.stdout.strip())
        raise ProjectGitError("<git>", message[:4_000])
    return result


def _try_git(
    root: Path,
    arguments: tuple[str, ...],
    *,
    timeout_seconds: float = GIT_TIMEOUT_SECONDS,
    allow_credentials: bool = False,
    allow_truncated_output: bool = False,
    budget: RequestBudget,
) -> GitCompletedProcess:
    git = shutil.which("git") or "git"
    command_parts = [
        git,
        "-c",
        f"core.hooksPath={os.devnull}",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "credential.helper=",
    ]
    helpers = (
        _trusted_credential_helpers(git, root=root, budget=budget)
        if allow_credentials
        else ()
    )
    if allow_credentials:
        for helper in helpers:
            command_parts.extend(("-c", f"credential.helper={helper}"))
    command_parts.extend(arguments)
    command = tuple(command_parts)
    with tempfile.NamedTemporaryFile(
        prefix=".codex-remote-git-",
        suffix=".config",
        delete=False,
    ) as policy_file:
        policy_path = Path(policy_file.name)
    try:
        try:
            _write_git_policy(policy_path, root, helpers)
        except OSError as exc:
            raise ProjectGitError(
                "<git>", "Git policy file could not be written"
            ) from exc
        environment = _safe_git_environment()
        environment["GIT_CONFIG_GLOBAL"] = str(policy_path)
        effective_timeout = budget.remaining(timeout_seconds)
        completed = run_owned_bounded_process(
            command,
            cwd=root,
            env=environment,
            timeout_seconds=effective_timeout,
            max_stream_bytes=MAX_GIT_CAPTURE_BYTES,
            cancel_event=budget.cancel_event,
        )
        output_truncated = completed.stdout_truncated or completed.stderr_truncated
        if output_truncated and not allow_truncated_output:
            raise ProjectGitError(
                "<git>", "Git command output exceeded the safe capture limit"
            )
        stdout = completed.stdout.decode("utf-8", errors="replace")
        stderr = completed.stderr.decode("utf-8", errors="replace")
        return GitCompletedProcess(
            args=command,
            returncode=completed.returncode,
            stdout=stdout,
            stderr=stderr,
            truncated=output_truncated,
        )
    except FileNotFoundError as exc:
        raise ProjectGitError("<git>", "Git executable was not found") from exc
    except subprocess.TimeoutExpired as exc:
        try:
            budget.ensure_active()
        except RequestDeadlineExpired:
            raise
        raise ProjectGitError("<git>", "Git command timed out") from exc
    except RemoteProcessCancelled as exc:
        try:
            budget.ensure_active()
        except RequestDeadlineExpired:
            raise
        raise ProjectGitError("<git>", "Git command was cancelled") from exc
    finally:
        policy_path.unlink(missing_ok=True)


def _safe_git_environment() -> dict[str, str]:
    environment = {
        key: value
        for key in GIT_ENVIRONMENT_KEYS
        if (value := os.environ.get(key)) is not None
    }
    environment["GIT_TERMINAL_PROMPT"] = "0"
    ssh = shutil.which("ssh")
    if ssh is not None:
        environment["GIT_SSH_COMMAND"] = f'"{ssh}"'
        environment["GIT_SSH_VARIANT"] = "ssh"
    true_command = shutil.which("true")
    if true_command is not None:
        environment["GIT_ASKPASS"] = true_command
        environment["SSH_ASKPASS"] = true_command
    return environment


def _trusted_credential_helpers(
    git: str,
    *,
    root: Path,
    budget: RequestBudget,
) -> tuple[str, ...]:
    helpers: list[str] = []
    for scope in ("--system", "--global"):
        timeout_seconds = budget.remaining(10)
        try:
            result = run_owned_bounded_process(
                (git, "config", scope, "--get-all", "credential.helper"),
                cwd=root,
                env=_safe_git_environment(),
                timeout_seconds=timeout_seconds,
                max_stream_bytes=MAX_GIT_CAPTURE_BYTES,
                cancel_event=budget.cancel_event,
            )
        except FileNotFoundError as exc:
            raise ProjectGitError(
                "<git>",
                "Git executable was not found",
            ) from exc
        except subprocess.TimeoutExpired as exc:
            try:
                budget.ensure_active()
            except RequestDeadlineExpired:
                raise
            raise ProjectGitError(
                "<git>",
                "Git credential helper lookup timed out",
            ) from exc
        except RemoteProcessCancelled as exc:
            try:
                budget.ensure_active()
            except RequestDeadlineExpired:
                raise
            raise ProjectGitError(
                "<git>", "Git credential helper lookup was cancelled"
            ) from exc
        if result.stdout_truncated or result.stderr_truncated:
            raise ProjectGitError("<git>", "Git credential helper output was too large")
        if result.returncode not in (0, 1):
            continue
        for helper in result.stdout.decode("utf-8", errors="replace").splitlines():
            value = helper.strip()
            if (
                value
                and not value.startswith("!")
                and SAFE_CREDENTIAL_HELPER.fullmatch(value) is not None
            ):
                helpers.append(value)
    return tuple(dict.fromkeys(helpers))


def _write_git_policy(
    path: Path,
    root: Path,
    helpers: tuple[str, ...],
) -> None:
    lines = (
        "[safe]",
        f"\tdirectory = {root.as_posix()}",
        "[credential]",
        "\thelper =",
        *(f"\thelper = {helper}" for helper in helpers),
        "",
    )
    _ = path.write_text("\n".join(lines), encoding="utf-8")


def _visible_paths(root: Path, paths: tuple[str, ...]) -> tuple[str, ...]:
    access = ProjectFileAccess(root)
    visible: list[str] = []
    for path in paths:
        try:
            _ = access.resolve_path(path, require_file=False)
        except ProjectFileError:
            continue
        visible.append(path)
    return tuple(visible)

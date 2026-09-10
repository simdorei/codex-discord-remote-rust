from __future__ import annotations

import difflib
import subprocess
from pathlib import Path
from typing import Final, Protocol

from codex_remote_mcp_files import (
    MAX_FILE_BYTES,
    ProjectFileAccess,
    ProjectFileError,
    ProjectFileLimitError,
)
from codex_remote_mcp_redaction import redact
from simdorei_mcp_common.operation_outputs import (
    DiffFile,
    RepoDiffOutput,
)
from simdorei_mcp_common.request_deadlines import RequestBudget

MAX_GIT_OUTPUT: Final = 200_000
MAX_UNTRACKED_FILES: Final = 10_000


class GitRunner(Protocol):
    def __call__(
        self,
        root: Path,
        arguments: tuple[str, ...],
        *,
        timeout_seconds: float = 120,
    ) -> subprocess.CompletedProcess[str]: ...


def build_repo_diff(
    root: Path,
    run_git: GitRunner,
    run_patch_git: GitRunner | None = None,
    *,
    budget: RequestBudget,
) -> RepoDiffOutput:
    """Return tracked and untracked changes as one bounded report."""
    budget.ensure_active()
    tracked_paths = _visible_tracked_paths(root, run_git, budget=budget)
    numeric = (
        run_git(root, ("diff", "--numstat", "HEAD", "--", *tracked_paths)).stdout
        if tracked_paths
        else ""
    )
    tracked_files = tuple(_parse_numstat(numeric))
    patch_runner = run_patch_git or run_git
    tracked_result = (
        patch_runner(
            root,
            ("diff", "--no-ext-diff", "HEAD", "--", *tracked_paths),
        )
        if tracked_paths
        else None
    )
    tracked_patch = tracked_result.stdout if tracked_result is not None else ""
    tracked_truncated = bool(
        getattr(tracked_result, "truncated", False)
        if tracked_result is not None
        else False
    )
    untracked_files, untracked_patch, untracked_truncated = _untracked_diff(
        root,
        run_git,
        budget=budget,
    )
    files = (*tracked_files, *untracked_files)
    patch = f"{tracked_patch}{untracked_patch}"
    truncated = tracked_truncated or untracked_truncated or len(patch) > MAX_GIT_OUTPUT
    total_added = sum(item.added for item in files)
    total_removed = sum(item.removed for item in files)
    return RepoDiffOutput(
        files=tuple(files),
        summary=f"{len(files)} file(s) changed, +{total_added}/-{total_removed}",
        patch=redact(patch[:MAX_GIT_OUTPUT]),
        truncated=truncated,
    )


def _untracked_diff(
    root: Path,
    run_git: GitRunner,
    *,
    budget: RequestBudget,
) -> tuple[tuple[DiffFile, ...], str, bool]:
    raw_paths = run_git(
        root,
        ("ls-files", "--others", "--exclude-standard", "-z"),
    ).stdout
    access = ProjectFileAccess(root)
    files: list[DiffFile] = []
    patches: list[str] = []
    retained_patch_chars = 0
    truncated = False
    observed = 0
    for path in (value for value in raw_paths.split("\0") if value):
        budget.ensure_active()
        observed += 1
        if observed > MAX_UNTRACKED_FILES:
            raise ProjectFileLimitError(
                "<git>",
                f"untracked file count exceeds {MAX_UNTRACKED_FILES}",
            )
        try:
            raw = access.read_bytes(path, max_bytes=MAX_FILE_BYTES)
        except ProjectFileError:
            continue
        budget.ensure_active()
        if b"\0" in raw:
            files.append(DiffFile(path=path, added=0, removed=0))
            retained_patch_chars, truncated = _append_bounded_patch(
                patches,
                f"Binary untracked file: {path}\n",
                retained_patch_chars,
                truncated,
            )
            continue
        try:
            lines = raw.decode("utf-8").splitlines(keepends=True)
        except UnicodeDecodeError:
            files.append(DiffFile(path=path, added=0, removed=0))
            retained_patch_chars, truncated = _append_bounded_patch(
                patches,
                f"Binary untracked file: {path}\n",
                retained_patch_chars,
                truncated,
            )
            continue
        files.append(DiffFile(path=path, added=len(lines), removed=0))
        patch = "".join(
            difflib.unified_diff(
                (),
                lines,
                fromfile="/dev/null",
                tofile=f"b/{path}",
            )
        )
        budget.ensure_active()
        retained_patch_chars, truncated = _append_bounded_patch(
            patches,
            patch,
            retained_patch_chars,
            truncated,
        )
    return tuple(files), "".join(patches), truncated


def _append_bounded_patch(
    patches: list[str],
    patch: str,
    retained_chars: int,
    already_truncated: bool,
) -> tuple[int, bool]:
    remaining = max(0, MAX_GIT_OUTPUT + 1 - retained_chars)
    if remaining:
        patches.append(patch[:remaining])
    retained = retained_chars + min(len(patch), remaining)
    return retained, already_truncated or len(patch) > remaining


def _parse_numstat(raw: str) -> tuple[DiffFile, ...]:
    files: list[DiffFile] = []
    for line in raw.splitlines():
        parts = line.split("\t", maxsplit=2)
        if len(parts) != 3:
            continue
        added = int(parts[0]) if parts[0].isdigit() else 0
        removed = int(parts[1]) if parts[1].isdigit() else 0
        files.append(DiffFile(path=parts[2], added=added, removed=removed))
    return tuple(files)


def _visible_tracked_paths(
    root: Path,
    run_git: GitRunner,
    *,
    budget: RequestBudget,
) -> tuple[str, ...]:
    raw = run_git(
        root,
        ("diff", "--name-only", "-z", "HEAD", "--"),
    ).stdout
    access = ProjectFileAccess(root)
    paths: list[str] = []
    for path in (value for value in raw.split("\0") if value):
        budget.ensure_active()
        try:
            _ = access.resolve_path(path, require_file=False)
        except ProjectFileError:
            continue
        paths.append(path)
    return tuple(paths)

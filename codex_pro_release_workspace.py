from __future__ import annotations

import hashlib
from pathlib import Path

from codex_pro_release_checks_types import CommandRunner


def git_revision(repo_root: Path, command_runner: CommandRunner) -> str:
    outcome = command_runner(("git", "rev-parse", "HEAD"), repo_root)
    return outcome.stdout.strip() if outcome.returncode == 0 else ""


def workspace_state(repo_root: Path, command_runner: CommandRunner) -> str:
    outcome = command_runner(("git", "status", "--porcelain"), repo_root)
    if outcome.returncode != 0:
        return "unavailable"
    return "dirty" if outcome.stdout.strip() else "clean"


def workspace_digest(
    repo_root: Path,
    command_runner: CommandRunner,
) -> str | None:
    tracked = command_runner(
        ("git", "diff", "--binary", "--no-ext-diff", "HEAD", "--"),
        repo_root,
    )
    untracked = command_runner(
        ("git", "ls-files", "--others", "--exclude-standard", "-z"),
        repo_root,
    )
    if tracked.returncode != 0 or untracked.returncode != 0:
        return None
    digest = hashlib.sha256(tracked.stdout.encode("utf-8"))
    try:
        for relative_raw in sorted(filter(None, untracked.stdout.split("\0"))):
            relative = Path(relative_raw)
            path = repo_root / relative
            if path.is_symlink():
                return None
            resolved = path.resolve(strict=True)
            if not resolved.is_relative_to(repo_root) or not resolved.is_file():
                return None
            relative_bytes = relative.as_posix().encode("utf-8")
            digest.update(len(relative_bytes).to_bytes(8, "big"))
            digest.update(relative_bytes)
            with resolved.open("rb") as source:
                while chunk := source.read(1024 * 1024):
                    digest.update(chunk)
    except OSError:
        return None
    return digest.hexdigest()

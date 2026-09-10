from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class CommandSandboxError(Exception):
    reason: str

    def __str__(self) -> str:
        return self.reason


def sandbox_arguments(root: Path, arguments: tuple[str, ...]) -> tuple[str, ...]:
    """Wrap a fixed project command in Codex's workspace-only sandbox."""
    prefix = _codex_prefix()
    if prefix is None:
        raise CommandSandboxError(
            "Codex CLI is required to run project commands safely"
        )
    return (
        *prefix,
        "sandbox",
        "-C",
        str(root),
        "-P",
        ":workspace",
        "--sandbox-state-disable-network",
        "--",
        *arguments,
    )


def _codex_prefix() -> tuple[str, ...] | None:
    candidate = os.environ.get("CODEX_EXE") or shutil.which("codex")
    if candidate is None:
        return None
    path = Path(candidate)
    if os.name != "nt" or path.suffix.casefold() == ".exe":
        return (str(path),)
    node = shutil.which("node")
    script = (
        path.parent
        / "node_modules"
        / "@openai"
        / "codex"
        / "bin"
        / "codex.js"
    )
    if node is not None and script.is_file():
        return (node, str(script))
    return None

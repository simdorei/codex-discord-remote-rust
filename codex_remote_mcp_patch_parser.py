from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from codex_remote_mcp_files import ProjectFileError


class PatchFormatError(ProjectFileError):
    """Raised when a Codex patch envelope is malformed."""


@dataclass(frozen=True, slots=True)
class PatchLine:
    kind: Literal["context", "add", "remove"]
    text: str


@dataclass(frozen=True, slots=True)
class PatchHunk:
    lines: tuple[PatchLine, ...]


@dataclass(frozen=True, slots=True)
class AddOperation:
    action: Literal["add"]
    path: str
    content: str


@dataclass(frozen=True, slots=True)
class DeleteOperation:
    action: Literal["delete"]
    path: str


@dataclass(frozen=True, slots=True)
class UpdateOperation:
    action: Literal["update"]
    path: str
    destination: str | None
    hunks: tuple[PatchHunk, ...]


PatchOperation = AddOperation | DeleteOperation | UpdateOperation


def parse_patch(patch: str) -> tuple[PatchOperation, ...]:
    """Parse one Codex patch envelope into typed file operations."""
    lines = patch.splitlines()
    if not lines or lines[0] != "*** Begin Patch":
        raise PatchFormatError("<patch>", "missing *** Begin Patch")
    if lines[-1] != "*** End Patch":
        raise PatchFormatError("<patch>", "missing *** End Patch")
    operations: list[PatchOperation] = []
    index = 1
    while index < len(lines) - 1:
        line = lines[index]
        if line.startswith("*** Add File: "):
            path = line.removeprefix("*** Add File: ").strip()
            index, content = _parse_add(lines, index + 1)
            operations.append(AddOperation(action="add", path=path, content=content))
            continue
        if line.startswith("*** Delete File: "):
            path = line.removeprefix("*** Delete File: ").strip()
            operations.append(DeleteOperation(action="delete", path=path))
            index += 1
            continue
        if line.startswith("*** Update File: "):
            path = line.removeprefix("*** Update File: ").strip()
            index, destination, hunks = _parse_update(lines, index + 1)
            operations.append(
                UpdateOperation(
                    action="update",
                    path=path,
                    destination=destination,
                    hunks=hunks,
                )
            )
            continue
        raise PatchFormatError("<patch>", f"unexpected line: {line}")
    if not operations:
        raise PatchFormatError("<patch>", "patch contains no file operations")
    return tuple(operations)


def _parse_add(lines: list[str], index: int) -> tuple[int, str]:
    content: list[str] = []
    while index < len(lines) - 1 and not lines[index].startswith("*** "):
        line = lines[index]
        if not line.startswith("+"):
            raise PatchFormatError("<patch>", "added file lines must start with +")
        content.append(line[1:])
        index += 1
    return index, "\n".join(content) + ("\n" if content else "")


def _parse_update(
    lines: list[str],
    index: int,
) -> tuple[int, str | None, tuple[PatchHunk, ...]]:
    destination: str | None = None
    if index < len(lines) - 1 and lines[index].startswith("*** Move to: "):
        destination = lines[index].removeprefix("*** Move to: ").strip()
        index += 1
    hunks: list[PatchHunk] = []
    while index < len(lines) - 1 and not lines[index].startswith("*** "):
        if not lines[index].startswith("@@"):
            raise PatchFormatError("<patch>", "update content must start with @@")
        index += 1
        hunk_lines: list[PatchLine] = []
        while (
            index < len(lines) - 1
            and not lines[index].startswith("@@")
            and not lines[index].startswith("*** ")
        ):
            raw = lines[index]
            if not raw or raw[0] not in " +-":
                raise PatchFormatError("<patch>", "patch lines need space, +, or -")
            kind: Literal["context", "add", "remove"]
            if raw[0] == " ":
                kind = "context"
            elif raw[0] == "+":
                kind = "add"
            elif raw[0] == "-":
                kind = "remove"
            else:
                raise PatchFormatError(
                    "<patch>",
                    "patch lines need space, +, or -",
                )
            hunk_lines.append(PatchLine(kind=kind, text=raw[1:]))
            index += 1
        hunks.append(PatchHunk(lines=tuple(hunk_lines)))
    if destination is None and not hunks:
        raise PatchFormatError("<patch>", "update requires a hunk or move destination")
    return index, destination, tuple(hunks)

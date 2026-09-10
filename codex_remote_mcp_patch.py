# pyright: reportUnnecessaryComparison=false
from __future__ import annotations

import hashlib
from dataclasses import dataclass
from difflib import SequenceMatcher
from pathlib import Path

from codex_remote_mcp_checkpoint_transaction import (
    CheckpointTarget,
    CheckpointTransaction,
    checkpoint_transaction,
)
from codex_remote_mcp_checkpoints import (
    begin_checkpoint,
    finish_checkpoint,
)
from codex_remote_mcp_files import (
    FileConflictError,
    MAX_FILE_BYTES,
    ProjectFileAccess,
)
from simdorei_mcp_common.operation_outputs import FileApplyPatchOutput, PatchEntry
from simdorei_mcp_common.operation_requests import FileApplyPatchRequest, FileChange
from simdorei_mcp_common.request_deadlines import RequestBudget


@dataclass(frozen=True, slots=True)
class PlannedMutation:
    entry: PatchEntry
    source: CheckpointTarget
    destination: CheckpointTarget | None
    content: str | None
    expected_sha256: str | None


def apply_project_patch(
    root: Path,
    request: FileApplyPatchRequest,
    *,
    budget: RequestBudget,
) -> FileApplyPatchOutput:
    """Apply a bounded Codex patch with SHA checks and rollback checkpoints."""
    access = ProjectFileAccess(root)
    planned = tuple(
        _plan_mutation(access, change) for change in request.changes
    )
    checkpoint_targets = _checkpoint_targets(planned)
    budget.ensure_active()
    draft = begin_checkpoint(root, "patch", checkpoint_targets)
    with checkpoint_transaction(draft) as transaction:
        _apply_mutations(access, planned, transaction, budget)
        checkpoint_id = finish_checkpoint(draft)
    return FileApplyPatchOutput(
        applied=tuple(mutation.entry for mutation in planned),
        checkpoint_id=checkpoint_id,
    )


def _plan_mutation(
    access: ProjectFileAccess,
    change: FileChange,
) -> PlannedMutation:
    match change:
        case FileChange(action="create", path=path, content=content):
            assert content is not None
            target = access.resolve_path(path, require_file=False)
            if access.file_exists(path):
                raise FileConflictError(path, "added file already exists")
            return PlannedMutation(
                entry=PatchEntry(
                    path=path,
                    action="add",
                    added_lines=len(content.splitlines()),
                    removed_lines=0,
                ),
                source=CheckpointTarget(path=path, absolute_path=target),
                destination=None,
                content=content,
                expected_sha256=None,
            )
        case FileChange(
            action="delete",
            path=path,
            expected_sha256=expected_sha256,
        ):
            target = access.resolve_path(path)
            expected = _require_hash(access, path, expected_sha256)
            return PlannedMutation(
                entry=PatchEntry(
                    path=path,
                    action="delete",
                    added_lines=0,
                    removed_lines=0,
                ),
                source=CheckpointTarget(path=path, absolute_path=target),
                destination=None,
                content=None,
                expected_sha256=expected,
            )
        case FileChange(
            action="update",
            path=path,
            content=content,
            expected_sha256=expected_sha256,
        ):
            assert content is not None
            source = access.resolve_path(path)
            expected = _require_hash(access, path, expected_sha256)
            original = access.read_bytes(
                path,
                max_bytes=MAX_FILE_BYTES,
            ).decode("utf-8")
            added, removed = _count_content_delta(original, content)
            return PlannedMutation(
                entry=PatchEntry(
                    path=path,
                    action="update",
                    added_lines=added,
                    removed_lines=removed,
                ),
                source=CheckpointTarget(path=path, absolute_path=source),
                destination=None,
                content=content,
                expected_sha256=expected,
            )
        case FileChange(
            action="move",
            path=path,
            content=content,
            destination=destination,
            expected_sha256=expected_sha256,
        ):
            assert destination is not None
            source = access.resolve_path(path)
            expected = _require_hash(access, path, expected_sha256)
            original = access.read_bytes(
                path,
                max_bytes=MAX_FILE_BYTES,
            ).decode("utf-8")
            moved_content = original if content is None else content
            target = access.resolve_path(destination, require_file=False)
            if access.file_exists(destination):
                raise FileConflictError(
                    destination, "move destination already exists"
                )
            added, removed = _count_content_delta(original, moved_content)
            return PlannedMutation(
                entry=PatchEntry(
                    path=path,
                    action="move",
                    destination=destination,
                    added_lines=added,
                    removed_lines=removed,
                ),
                source=CheckpointTarget(path=path, absolute_path=source),
                destination=CheckpointTarget(
                    path=destination,
                    absolute_path=target,
                ),
                content=moved_content,
                expected_sha256=expected,
            )
        case _:
            raise ValueError(f"unsupported file change action: {change.action}")


def _require_hash(
    access: ProjectFileAccess,
    path: str,
    expected: str | None,
) -> str:
    if expected is None:
        raise FileConflictError(path, "existing files require a precondition hash")
    current = hashlib.sha256(
        access.read_bytes(path, max_bytes=MAX_FILE_BYTES)
    ).hexdigest()
    if current != expected:
        raise FileConflictError(path, "file changed since it was read")
    return expected


def _count_content_delta(original: str, updated: str) -> tuple[int, int]:
    matcher = SequenceMatcher(a=original.splitlines(), b=updated.splitlines())
    added = 0
    removed = 0
    for tag, old_start, old_end, new_start, new_end in matcher.get_opcodes():
        if tag in {"replace", "delete"}:
            removed += old_end - old_start
        if tag in {"replace", "insert"}:
            added += new_end - new_start
    return added, removed


def _checkpoint_targets(
    mutations: tuple[PlannedMutation, ...],
) -> tuple[CheckpointTarget, ...]:
    unique: dict[str, CheckpointTarget] = {}
    for mutation in mutations:
        unique[mutation.source.path] = mutation.source
        if mutation.destination is not None:
            unique[mutation.destination.path] = mutation.destination
    return tuple(unique.values())


def _apply_mutations(
    access: ProjectFileAccess,
    mutations: tuple[PlannedMutation, ...],
    transaction: CheckpointTransaction,
    budget: RequestBudget,
) -> None:
    for mutation in mutations:
        if mutation.expected_sha256 is not None:
            current = hashlib.sha256(
                access.read_bytes(
                    mutation.source.path,
                    max_bytes=MAX_FILE_BYTES,
                )
            ).hexdigest()
            if current != mutation.expected_sha256:
                raise FileConflictError(
                    mutation.source.path,
                    "file changed since the patch was planned",
                )
        if mutation.entry.action == "delete":
            budget.ensure_active()
            access.delete_file(
                mutation.source.path,
                expected_sha256=mutation.expected_sha256 or "",
            )
            transaction.record_delete(mutation.source.path)
            continue
        if mutation.destination is not None:
            budget.ensure_active()
            written = access.write_file(
                mutation.destination.path,
                mutation.content or "",
                expected_sha256=None,
            )
            transaction.record_write(mutation.destination.path, written.sha256)
            budget.ensure_active()
            access.delete_file(
                mutation.source.path,
                expected_sha256=mutation.expected_sha256 or "",
            )
            transaction.record_delete(mutation.source.path)
            continue
        budget.ensure_active()
        written = access.write_file(
            mutation.source.path,
            mutation.content or "",
            expected_sha256=mutation.expected_sha256,
        )
        transaction.record_write(mutation.source.path, written.sha256)

from __future__ import annotations

import hashlib
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path
from typing import final

from codex_remote_mcp_files import FileConflictError, ProjectFileAccess


@dataclass(frozen=True, slots=True)
class CheckpointTarget:
    path: str
    absolute_path: Path


@dataclass(frozen=True, slots=True)
class BeforeSnapshot:
    target: CheckpointTarget
    existed: bool
    content: bytes


@dataclass(frozen=True, slots=True)
class CheckpointDraft:
    root: Path
    checkpoint_id: str
    created_at: str
    reason: str
    max_target_bytes: int
    snapshots: tuple[BeforeSnapshot, ...]


@dataclass(frozen=True, slots=True)
class MutationState:
    exists: bool
    sha256: str | None


class CheckpointCoverageError(RuntimeError):
    """Raised when a transaction records a path outside its checkpoint."""


@final
class CheckpointTransaction:
    """Track only states produced by the mutation this transaction owns."""

    __slots__ = ("_produced", "draft")

    def __init__(self, draft: CheckpointDraft) -> None:
        self.draft: CheckpointDraft = draft
        self._produced: dict[str, MutationState] = {}

    def record_write(self, path: str, sha256: str) -> None:
        self._require_target(path)
        self._produced[path] = MutationState(exists=True, sha256=sha256)

    def record_delete(self, path: str) -> None:
        self._require_target(path)
        self._produced[path] = MutationState(exists=False, sha256=None)

    def produced_state(self, path: str) -> MutationState | None:
        return self._produced.get(path)

    def _require_target(self, path: str) -> None:
        if not any(snapshot.target.path == path for snapshot in self.draft.snapshots):
            raise CheckpointCoverageError(f"checkpoint does not cover {path}")


def rollback_checkpoint(transaction: CheckpointTransaction) -> None:
    """Restore owned mutations without overwriting a concurrent writer."""
    access = ProjectFileAccess(transaction.draft.root)
    snapshots = tuple(
        snapshot
        for snapshot in transaction.draft.snapshots
        if transaction.produced_state(snapshot.target.path) is not None
    )
    for snapshot in snapshots:
        state = transaction.produced_state(snapshot.target.path)
        if state is None:
            continue
        _validate_produced_state(
            access,
            snapshot.target.path,
            state,
            max_bytes=transaction.draft.max_target_bytes,
        )
    for snapshot in reversed(snapshots):
        path = snapshot.target.path
        state = transaction.produced_state(path)
        if state is None:
            continue
        if snapshot.existed:
            _ = access.write_bytes(
                path,
                snapshot.content,
                expected_sha256=state.sha256,
            )
        elif state.exists:
            access.delete_file(path, expected_sha256=state.sha256 or "")


@contextmanager
def checkpoint_transaction(
    draft: CheckpointDraft,
) -> Generator[CheckpointTransaction]:
    """Rollback recorded mutations if a write or persistence step fails."""
    transaction = CheckpointTransaction(draft)
    completed = False
    try:
        yield transaction
        completed = True
    finally:
        if not completed:
            rollback_checkpoint(transaction)


def _validate_produced_state(
    access: ProjectFileAccess,
    path: str,
    state: MutationState,
    *,
    max_bytes: int,
) -> None:
    current_exists = access.file_exists(path)
    if current_exists != state.exists:
        raise FileConflictError(path, "file changed during rollback")
    if not current_exists:
        return
    current_sha256 = hashlib.sha256(
        access.read_bytes(path, max_bytes=max_bytes)
    ).hexdigest()
    if current_sha256 != state.sha256:
        raise FileConflictError(path, "file changed during rollback")

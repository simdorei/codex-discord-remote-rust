from __future__ import annotations

import base64
import difflib
import hashlib
import os
import re
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import ClassVar, Final

from pydantic import BaseModel, ConfigDict, Field

from codex_remote_mcp_checkpoint_transaction import (
    BeforeSnapshot,
    CheckpointDraft,
    CheckpointTarget,
    checkpoint_transaction,
)
from codex_remote_mcp_file_store import (
    ProjectFileStore,
    ProjectFileStoreError,
    ProjectFileStoreReadLimit,
)
from codex_remote_mcp_files import (
    FileConflictError,
    ProjectFileAccess,
    ProjectFileLimitError,
    ProjectFileSizeError,
    UnsafeProjectPathError,
)
from codex_remote_mcp_redaction import redact
from simdorei_mcp_common.operation_outputs import CheckpointEntry
from simdorei_mcp_common.request_deadlines import RequestBudget

CHECKPOINT_DIRECTORY: Final = ".codex-remote-mcp/checkpoints"
MAX_CHECKPOINT_BYTES: Final = 10_485_760
MAX_CHECKPOINT_RECORD_BYTES: Final = 33_554_432
MAX_CHECKPOINT_SCAN_CANDIDATES: Final = 10_000
MAX_CHECKPOINT_RESULTS: Final = 50
MAX_CHECKPOINT_RETAINED: Final = 500
MAX_CHECKPOINT_STORAGE_BYTES: Final = 536_870_912
CHECKPOINT_FILE_PATTERN: Final = re.compile(r"^cp_[a-f0-9]{16}\.json$")
_CHECKPOINT_LOCK: Final = threading.Lock()


class StoredSnapshot(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    path: str
    before_exists: bool
    before_base64: str
    after_exists: bool
    after_base64: str


class StoredCheckpoint(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="forbid")

    checkpoint_id: str = Field(pattern=r"^cp_[a-f0-9]{16}$")
    created_at: str
    reason: str
    snapshots: tuple[StoredSnapshot, ...]


def begin_checkpoint(
    root: Path,
    reason: str,
    targets: tuple[CheckpointTarget, ...],
    *,
    max_target_bytes: int = MAX_CHECKPOINT_BYTES,
) -> CheckpointDraft:
    """Capture pre-mutation bytes for a bounded set of validated paths."""
    snapshots: list[BeforeSnapshot] = []
    access = ProjectFileAccess(root)
    total_bytes = 0
    for target in targets:
        existed = access.file_exists(target.path)
        remaining = MAX_CHECKPOINT_BYTES - total_bytes
        try:
            content = (
                access.read_bytes(
                    target.path,
                    max_bytes=min(max_target_bytes, remaining),
                )
                if existed
                else b""
            )
        except ProjectFileSizeError as exc:
            raise ProjectFileSizeError(
                target.path,
                f"checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes",
            ) from exc
        total_bytes += len(content)
        if total_bytes > MAX_CHECKPOINT_BYTES:
            raise ProjectFileSizeError(
                target.path,
                f"checkpoint exceeds {MAX_CHECKPOINT_BYTES} bytes",
            )
        snapshots.append(
            BeforeSnapshot(
                target=target,
                existed=existed,
                content=content,
            )
        )
    timestamp = datetime.now(UTC)
    seed = "\0".join(target.path for target in targets)
    digest = hashlib.sha256(
        f"{timestamp.isoformat()}\0{reason}\0{seed}".encode()
    ).hexdigest()[:16]
    return CheckpointDraft(
        root=root,
        checkpoint_id=f"cp_{digest}",
        created_at=timestamp.isoformat(),
        reason=reason,
        max_target_bytes=max_target_bytes,
        snapshots=tuple(snapshots),
    )


def finish_checkpoint(draft: CheckpointDraft) -> str:
    """Persist the before/after record after a successful mutation."""
    stored: list[StoredSnapshot] = []
    access = ProjectFileAccess(draft.root)
    total_bytes = 0
    for snapshot in draft.snapshots:
        after_exists = access.file_exists(snapshot.target.path)
        remaining = (
            MAX_CHECKPOINT_BYTES * 2
            - total_bytes
            - len(snapshot.content)
        )
        try:
            after = (
                access.read_bytes(
                    snapshot.target.path,
                    max_bytes=min(draft.max_target_bytes, remaining),
                )
                if after_exists
                else b""
            )
        except ProjectFileSizeError as exc:
            raise ProjectFileSizeError(
                snapshot.target.path,
                "checkpoint before/after data is too large",
            ) from exc
        total_bytes += len(snapshot.content) + len(after)
        if total_bytes > MAX_CHECKPOINT_BYTES * 2:
            raise ProjectFileSizeError(
                snapshot.target.path,
                "checkpoint before/after data is too large",
            )
        stored.append(
            StoredSnapshot(
                path=snapshot.target.path,
                before_exists=snapshot.existed,
                before_base64=base64.b64encode(snapshot.content).decode("ascii"),
                after_exists=after_exists,
                after_base64=base64.b64encode(after).decode("ascii"),
            )
        )
    record = StoredCheckpoint(
        checkpoint_id=draft.checkpoint_id,
        created_at=draft.created_at,
        reason=draft.reason,
        snapshots=tuple(stored),
    )
    relative = Path(CHECKPOINT_DIRECTORY) / f"{draft.checkpoint_id}.json"
    serialized = record.model_dump_json().encode("utf-8")
    if len(serialized) > MAX_CHECKPOINT_RECORD_BYTES:
        raise ProjectFileSizeError(
            draft.checkpoint_id,
            "checkpoint record is too large",
        )
    store = ProjectFileStore(draft.root.resolve())
    with _CHECKPOINT_LOCK:
        _prune_checkpoint_storage(store, incoming_bytes=len(serialized))
        _ = store.write_bytes(
            relative,
            serialized,
            expected_sha256=None,
        )
    return draft.checkpoint_id


def list_checkpoints(
    root: Path,
    *,
    budget: RequestBudget,
) -> tuple[CheckpointEntry, ...]:
    """Return newest checkpoint metadata without exposing stored bytes."""
    try:
        directory = ProjectFileStore(root.resolve()).ensure(
            Path(CHECKPOINT_DIRECTORY),
            require_file=False,
        )
    except ProjectFileStoreError as exc:
        raise UnsafeProjectPathError(CHECKPOINT_DIRECTORY, str(exc)) from exc
    if not directory.is_dir():
        return ()
    candidates: list[tuple[int, str]] = []
    observed = 0
    try:
        with os.scandir(directory) as iterator:
            for candidate in iterator:
                budget.ensure_active()
                observed += 1
                if observed > MAX_CHECKPOINT_SCAN_CANDIDATES:
                    raise ProjectFileLimitError(
                        CHECKPOINT_DIRECTORY,
                        (
                            "checkpoint listing scanned too many candidates; "
                            f"limit is {MAX_CHECKPOINT_SCAN_CANDIDATES}"
                        ),
                    )
                if (
                    CHECKPOINT_FILE_PATTERN.fullmatch(candidate.name) is None
                    or not candidate.is_file(follow_symlinks=False)
                ):
                    continue
                modified_ns = candidate.stat(follow_symlinks=False).st_mtime_ns
                candidates.append((modified_ns, candidate.name))
    except ProjectFileLimitError:
        raise
    except OSError as exc:
        raise UnsafeProjectPathError(CHECKPOINT_DIRECTORY, str(exc)) from exc
    candidates.sort(reverse=True)
    entries = [
        _entry(
            _load_record(
                root,
                name.removesuffix(".json"),
                budget=budget,
            )
        )
        for _, name in candidates[:MAX_CHECKPOINT_RESULTS]
    ]
    return tuple(sorted(entries, key=lambda entry: entry.created_at, reverse=True))


def show_checkpoint(
    root: Path,
    checkpoint_id: str,
    *,
    budget: RequestBudget,
) -> tuple[CheckpointEntry, str]:
    """Return checkpoint metadata and a bounded unified diff."""
    record = _load_record(root, checkpoint_id, budget=budget)
    fragments = tuple(_snapshot_diff(snapshot) for snapshot in record.snapshots)
    patch = "\n".join(fragment for fragment in fragments if fragment)
    return _entry(record), redact(patch[:MAX_CHECKPOINT_BYTES])


def restore_checkpoint(
    root: Path,
    checkpoint_id: str,
    *,
    budget: RequestBudget,
) -> tuple[str, ...]:
    """Restore every checkpointed path to its before state."""
    budget.ensure_active()
    record = _load_record(root, checkpoint_id, budget=budget)
    access = ProjectFileAccess(root)
    snapshots = record.snapshots
    targets: list[CheckpointTarget] = []
    for snapshot in snapshots:
        absolute_path = access.resolve_path(snapshot.path, require_file=False)
        targets.append(CheckpointTarget(snapshot.path, absolute_path))
        current_exists = access.file_exists(snapshot.path)
        expected_after = base64.b64decode(
            snapshot.after_base64,
            validate=True,
        )
        if current_exists != snapshot.after_exists or (
            current_exists
            and access.read_bytes(
                snapshot.path,
                max_bytes=MAX_CHECKPOINT_BYTES,
            )
            != expected_after
        ):
            raise FileConflictError(
                snapshot.path,
                "file changed after checkpoint was created",
            )
    budget.ensure_active()
    rollback = begin_checkpoint(
        root,
        f"rollback failed restore {checkpoint_id}",
        tuple(targets),
    )
    restored: list[str] = []
    with checkpoint_transaction(rollback) as transaction:
        for snapshot in snapshots:
            current_exists = access.file_exists(snapshot.path)
            expected_after = base64.b64decode(
                snapshot.after_base64,
                validate=True,
            )
            if snapshot.before_exists:
                content = base64.b64decode(snapshot.before_base64, validate=True)
                expected = (
                    hashlib.sha256(expected_after).hexdigest()
                    if snapshot.after_exists
                    else None
                )
                budget.ensure_active()
                _ = access.write_bytes(
                    snapshot.path,
                    content,
                    expected_sha256=expected,
                )
                transaction.record_write(
                    snapshot.path,
                    hashlib.sha256(content).hexdigest(),
                )
            elif current_exists:
                budget.ensure_active()
                access.delete_file(
                    snapshot.path,
                    expected_sha256=hashlib.sha256(expected_after).hexdigest(),
                )
                transaction.record_delete(snapshot.path)
            restored.append(snapshot.path)
    return tuple(restored)


def _load_record(
    root: Path,
    checkpoint_id: str,
    *,
    budget: RequestBudget,
) -> StoredCheckpoint:
    if CHECKPOINT_FILE_PATTERN.fullmatch(f"{checkpoint_id}.json") is None:
        raise UnsafeProjectPathError(checkpoint_id, "invalid checkpoint id")
    relative = Path(CHECKPOINT_DIRECTORY) / f"{checkpoint_id}.json"
    budget.ensure_active()
    try:
        raw = ProjectFileStore(root.resolve()).read_bytes(
            relative,
            max_bytes=MAX_CHECKPOINT_RECORD_BYTES,
        )
    except ProjectFileStoreReadLimit as exc:
        raise ProjectFileSizeError(
            checkpoint_id,
            "checkpoint record is too large",
        ) from exc
    except ProjectFileStoreError as exc:
        raise UnsafeProjectPathError(checkpoint_id, str(exc)) from exc
    budget.ensure_active()
    return StoredCheckpoint.model_validate_json(raw)


def _prune_checkpoint_storage(
    store: ProjectFileStore,
    *,
    incoming_bytes: int,
) -> None:
    directory = store.ensure(
        Path(CHECKPOINT_DIRECTORY),
        require_file=False,
    )
    if not directory.is_dir():
        return
    candidates: list[tuple[int, str, int, Path]] = []
    observed = 0
    with os.scandir(directory) as iterator:
        for candidate in iterator:
            observed += 1
            if observed > MAX_CHECKPOINT_SCAN_CANDIDATES:
                raise ProjectFileLimitError(
                    CHECKPOINT_DIRECTORY,
                    "checkpoint retention scanned too many candidates",
                )
            if (
                CHECKPOINT_FILE_PATTERN.fullmatch(candidate.name) is None
                or not candidate.is_file(follow_symlinks=False)
            ):
                continue
            stat = candidate.stat(follow_symlinks=False)
            candidates.append(
                (
                    stat.st_mtime_ns,
                    candidate.name,
                    stat.st_size,
                    Path(candidate.path),
                )
            )
    candidates.sort(reverse=True)
    retained_count = 0
    retained_bytes = 0
    available_bytes = MAX_CHECKPOINT_STORAGE_BYTES - incoming_bytes
    available_count = MAX_CHECKPOINT_RETAINED - 1
    for _, _, size, path in candidates:
        if (
            retained_count < available_count
            and retained_bytes + size <= available_bytes
        ):
            retained_count += 1
            retained_bytes += size
            continue
        path.unlink(missing_ok=True)


def _entry(record: StoredCheckpoint) -> CheckpointEntry:
    return CheckpointEntry(
        checkpoint_id=record.checkpoint_id,
        created_at=record.created_at,
        reason=record.reason,
    )


def _snapshot_diff(snapshot: StoredSnapshot) -> str:
    before = base64.b64decode(snapshot.before_base64, validate=True)
    after = base64.b64decode(snapshot.after_base64, validate=True)
    try:
        before_text = before.decode("utf-8").splitlines(keepends=True)
        after_text = after.decode("utf-8").splitlines(keepends=True)
    except UnicodeDecodeError:
        return f"Binary file changed: {snapshot.path}"
    return "".join(
        difflib.unified_diff(
            before_text,
            after_text,
            fromfile=f"a/{snapshot.path}",
            tofile=f"b/{snapshot.path}",
        )
    )

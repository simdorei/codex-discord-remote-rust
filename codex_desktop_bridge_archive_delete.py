from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import TypeAlias, TypedDict

import codex_desktop_bridge_archive_backup as archive_backup
import codex_desktop_bridge_archive_scrub as archive_scrub
import codex_desktop_bridge_session_index as session_index
import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_records as thread_records
from codex_thread_models import ThreadInfo


CountRow: TypeAlias = tuple[int | None]


class DeleteArchivedThreadResult(TypedDict):
    backup_dir: Path
    backup_paths: list[Path]
    deleted_log_rows: int
    deleted_rollout_path: str
    bridge_state_scrubbed: list[str]
    global_state_scrubbed: list[str]
    session_index_removed: int


class ArchiveDeletePathOutsideArchiveError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(
            "Refusing to delete an archived thread whose rollout path is outside the archived_sessions directory."
        )


class ArchiveDeleteMissingThreadRecordError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("The archived thread no longer exists in the local state DB.")


class ArchiveDeleteActiveThreadError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Refusing to delete an active thread. Only archived threads can be deleted.")


class ArchiveDeleteConcurrentMutationError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Archived thread deletion aborted because the target row changed during deletion.")


class ArchiveDeleteThreadRecordStillPresentError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Archived thread row is still present after deletion.")


class ArchiveDeleteLogsStillPresentError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Archived thread logs are still present after deletion.")


class ArchiveDeleteRolloutFileStillPresentError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Archived rollout file is still present after deletion.")


def _connect_readonly(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def _connect_writable(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(path)


def delete_archived_thread_locally(thread: ThreadInfo) -> DeleteArchivedThreadResult:
    rollout_path = Path(session_index.strip_windows_extended_prefix(thread.rollout_path)).expanduser()
    if not archive_backup.is_path_within_directory(rollout_path, bridge_state.ARCHIVED_SESSIONS_DIR):
        raise ArchiveDeletePathOutsideArchiveError()

    record = thread_records.load_thread_record_by_id(thread.id)
    if record is None:
        raise ArchiveDeleteMissingThreadRecordError()
    _thread_record, is_archived = record
    if not is_archived:
        raise ArchiveDeleteActiveThreadError()

    backup_dir = archive_backup.create_archive_delete_backup_dir(thread.id)
    backup_paths = archive_backup.backup_archive_delete_inputs(backup_dir)

    with _connect_writable(bridge_state.STATE_DB_PATH) as conn:
        _ = conn.execute("BEGIN IMMEDIATE")
        _ = conn.execute(
            "\n".join(
                (
                    "DELETE FROM thread_spawn_edges",
                    "WHERE child_thread_id = ?",
                    "   OR parent_thread_id = ?",
                )
            ),
            (thread.id, thread.id),
        )
        deleted_rows = conn.execute("DELETE FROM threads WHERE id = ?", (thread.id,)).rowcount
        if deleted_rows != 1:
            conn.rollback()
            raise ArchiveDeleteConcurrentMutationError()
        conn.commit()

    deleted_log_rows = 0
    if bridge_state.LOG_DB_PATH.exists():
        with _connect_writable(bridge_state.LOG_DB_PATH) as conn:
            _ = conn.execute("BEGIN IMMEDIATE")
            deleted_log_rows = conn.execute("DELETE FROM logs WHERE thread_id = ?", (thread.id,)).rowcount
            conn.commit()

    bridge_state_scrubbed = archive_scrub.scrub_bridge_state_deleted_thread(thread.id)
    global_state_scrubbed = archive_scrub.scrub_global_state_deleted_thread(thread.id)
    session_index_removed = archive_scrub.scrub_session_index_deleted_thread(thread.id)

    deleted_rollout_path = ""
    if rollout_path.exists():
        rollout_path.unlink()
        deleted_rollout_path = str(rollout_path)

    if thread_records.load_thread_record_by_id(thread.id) is not None:
        raise ArchiveDeleteThreadRecordStillPresentError()

    remaining_log_rows = 0
    if bridge_state.LOG_DB_PATH.exists():
        with _connect_readonly(bridge_state.LOG_DB_PATH) as conn:
            rows: list[CountRow] = conn.execute(
                "SELECT COUNT(*) FROM logs WHERE thread_id = ?",
                (thread.id,),
            ).fetchall()
            remaining_log_rows = int(rows[0][0] or 0) if rows else 0
    if remaining_log_rows:
        raise ArchiveDeleteLogsStillPresentError()

    if rollout_path.exists():
        raise ArchiveDeleteRolloutFileStillPresentError()

    return {
        "backup_dir": backup_dir,
        "backup_paths": backup_paths,
        "deleted_log_rows": deleted_log_rows,
        "deleted_rollout_path": deleted_rollout_path or str(rollout_path),
        "bridge_state_scrubbed": bridge_state_scrubbed,
        "global_state_scrubbed": global_state_scrubbed,
        "session_index_removed": session_index_removed,
    }

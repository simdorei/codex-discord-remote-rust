from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

import codex_desktop_bridge_archive_commands as archive_commands
import codex_desktop_bridge_archive_delete as archive_delete
import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_records as thread_records
from codex_thread_models import ThreadInfo


class ArchiveCommandTests(unittest.TestCase):
    def test_run_archive_command_raises_typed_timeout_when_archived_state_missing(self) -> None:
        archive_calls: list[tuple[str, bool]] = []
        printed_lines: list[str] = []
        deps = _deps(
            archive_thread_with_lock_retry=lambda thread_id, kill_on_lock: archive_calls.append(
                (thread_id, kill_on_lock)
            ),
            wait_for_thread_record=lambda _thread_id, _archived, _timeout: None,
            print_line=printed_lines.append,
        )

        with self.assertRaisesRegex(
            archive_commands.ArchiveThreadStateTimeoutError,
            "thread/archive returned, but the thread did not appear as archived",
        ):
            archive_commands.run_archive_command(
                _thread(),
                no_kill_codex_on_lock=False,
                timeout=1.5,
                deps=deps,
            )

        self.assertEqual(archive_calls, [("thread-1", True)])
        self.assertEqual(printed_lines, [])

    def test_delete_archived_thread_raises_typed_path_error_for_outside_rollout(self) -> None:
        with TemporaryDirectory() as archive_dir, TemporaryDirectory() as outside_dir:
            original_archive_dir = bridge_state.ARCHIVED_SESSIONS_DIR
            try:
                bridge_state.ARCHIVED_SESSIONS_DIR = Path(archive_dir)
                with self.assertRaisesRegex(
                    archive_delete.ArchiveDeletePathOutsideArchiveError,
                    "outside the archived_sessions directory",
                ):
                    _ = archive_delete.delete_archived_thread_locally(
                        _thread_with_rollout(Path(outside_dir) / "session.jsonl")
                    )
            finally:
                bridge_state.ARCHIVED_SESSIONS_DIR = original_archive_dir

    def test_delete_archived_thread_raises_typed_missing_record_error(self) -> None:
        with TemporaryDirectory() as archive_dir:
            original_archive_dir = bridge_state.ARCHIVED_SESSIONS_DIR
            original_loader = thread_records.load_thread_record_by_id

            def missing_record(_thread_id: str) -> tuple[ThreadInfo, bool] | None:
                return None

            try:
                bridge_state.ARCHIVED_SESSIONS_DIR = Path(archive_dir)
                thread_records.load_thread_record_by_id = missing_record
                with self.assertRaisesRegex(
                    archive_delete.ArchiveDeleteMissingThreadRecordError,
                    "no longer exists in the local state DB",
                ):
                    _ = archive_delete.delete_archived_thread_locally(
                        _thread_with_rollout(Path(archive_dir) / "session.jsonl")
                    )
            finally:
                thread_records.load_thread_record_by_id = original_loader
                bridge_state.ARCHIVED_SESSIONS_DIR = original_archive_dir


def _deps(
    *,
    archive_thread_with_lock_retry: archive_commands.ArchiveThreadWithLockRetry | None = None,
    wait_for_thread_record: archive_commands.WaitForThreadRecord | None = None,
    print_line: archive_commands.PrintLine | None = None,
) -> archive_commands.ArchiveCommandDeps:
    return archive_commands.ArchiveCommandDeps(
        get_thread_busy_state=lambda _thread: "idle",
        describe_thread_busy_state=lambda state: f"busy: {state}",
        archive_thread_with_lock_retry=archive_thread_with_lock_retry or _archive_thread,
        wait_for_thread_record=wait_for_thread_record or _wait_for_archived_thread,
        get_selected_thread_id=lambda: None,
        set_selected_thread_id=lambda _thread_id: None,
        sync_session_index_with_state=lambda: 0,
        format_title_preview=lambda title: title,
        format_timestamp=lambda timestamp: str(timestamp),
        delete_archived_thread_locally=_delete_archived_thread_locally,
        print_line=print_line or _ignore_line,
    )


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path="session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


def _thread_with_rollout(rollout_path: Path) -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path=str(rollout_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


def _archive_thread(_thread_id: str, _kill_on_lock: bool) -> None:
    return None


def _wait_for_archived_thread(
    _thread_id: str,
    _archived: bool,
    _timeout: float,
) -> tuple[ThreadInfo, bool] | None:
    return (_thread(), True)


def _delete_archived_thread_locally(_thread: ThreadInfo) -> archive_delete.DeleteArchivedThreadResult:
    return {
        "backup_dir": Path("backup"),
        "backup_paths": [],
        "deleted_log_rows": 0,
        "deleted_rollout_path": "",
        "bridge_state_scrubbed": [],
        "global_state_scrubbed": [],
        "session_index_removed": 0,
    }


def _ignore_line(_line: str) -> None:
    return None


if __name__ == "__main__":
    _ = unittest.main()

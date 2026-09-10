from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Final

import codex_desktop_bridge_archive_delete as archive_delete
from codex_thread_models import ThreadInfo

ARCHIVE_STATE_TIMEOUT_MESSAGE: Final = (
    "thread/archive returned, but the thread did not appear as archived in local Codex state in time."
)


ArchiveThreadWithLockRetry = Callable[[str, bool], None]
DeleteArchivedThread = Callable[[ThreadInfo], archive_delete.DeleteArchivedThreadResult]
DescribeThreadBusyState = Callable[[str], str]
FormatTimestamp = Callable[[int], str]
FormatTitlePreview = Callable[[str], str]
GetSelectedThreadId = Callable[[], str | None]
GetThreadBusyState = Callable[[ThreadInfo], str]
PrintLine = Callable[[str], None]
SetSelectedThreadId = Callable[[str | None], None]
SyncSessionIndex = Callable[[], int | None]
WaitForThreadRecord = Callable[[str, bool, float], tuple[ThreadInfo, bool] | None]


@dataclass(frozen=True, slots=True)
class ArchiveCommandDeps:
    get_thread_busy_state: GetThreadBusyState
    describe_thread_busy_state: DescribeThreadBusyState
    archive_thread_with_lock_retry: ArchiveThreadWithLockRetry
    wait_for_thread_record: WaitForThreadRecord
    get_selected_thread_id: GetSelectedThreadId
    set_selected_thread_id: SetSelectedThreadId
    sync_session_index_with_state: SyncSessionIndex
    format_title_preview: FormatTitlePreview
    format_timestamp: FormatTimestamp
    delete_archived_thread_locally: DeleteArchivedThread
    print_line: PrintLine


class ArchiveThreadStateTimeoutError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(ARCHIVE_STATE_TIMEOUT_MESSAGE)


def run_archive_command(
    thread: ThreadInfo,
    *,
    no_kill_codex_on_lock: bool,
    timeout: float,
    deps: ArchiveCommandDeps,
) -> None:
    busy_state = deps.get_thread_busy_state(thread)
    if busy_state != "idle":
        raise RuntimeError(deps.describe_thread_busy_state(busy_state))

    deps.archive_thread_with_lock_retry(thread.id, not no_kill_codex_on_lock)
    archived_record = deps.wait_for_thread_record(thread.id, True, timeout)
    if archived_record is None:
        raise ArchiveThreadStateTimeoutError()
    archived_thread, _archived = archived_record

    if deps.get_selected_thread_id() == thread.id:
        deps.set_selected_thread_id(None)
        deps.print_line("selected_thread: cleared")

    _ = deps.sync_session_index_with_state()
    deps.print_line(f"archived_thread: {thread.id}")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"cwd: {thread.cwd}")
    deps.print_line(f"archived_rollout_path: {archived_thread.rollout_path}")
    deps.print_line("transport: local-sidecar thread/archive")


def run_delete_archive_command(
    thread: ThreadInfo,
    *,
    confirm: bool,
    deps: ArchiveCommandDeps,
) -> None:
    deps.print_line(f"thread_id: {thread.id}")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"cwd: {thread.cwd}")
    deps.print_line(f"archived_at: {deps.format_timestamp(thread.archived_at or thread.updated_at)}")
    deps.print_line(f"rollout_path: {thread.rollout_path}")
    if not confirm:
        deps.print_line("delete_mode: preview")
        deps.print_line(f"rerun: delete_archive --confirm {thread.id}")
        return

    result = deps.delete_archived_thread_locally(thread)
    _ = deps.sync_session_index_with_state()
    deps.print_line("delete_mode: confirmed")
    deps.print_line(f"deleted_log_rows: {result['deleted_log_rows']}")
    deps.print_line(f"deleted_rollout_path: {result['deleted_rollout_path']}")
    deps.print_line(f"backup_dir: {result['backup_dir']}")
    if result["backup_paths"]:
        deps.print_line("backup_files:")
        for path in result["backup_paths"]:
            deps.print_line(f"- {path}")
    if result["bridge_state_scrubbed"]:
        deps.print_line(f"bridge_state_scrubbed: {', '.join(result['bridge_state_scrubbed'])}")
    if result["global_state_scrubbed"]:
        deps.print_line(f"global_state_scrubbed: {', '.join(result['global_state_scrubbed'])}")
    deps.print_line(f"session_index_removed: {result['session_index_removed']}")

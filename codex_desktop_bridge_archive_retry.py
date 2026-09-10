from __future__ import annotations

import time
from collections.abc import Callable

import codex_desktop_bridge_file_backup as file_backup
import codex_desktop_bridge_formatting as bridge_formatting
from codex_desktop_bridge_sidecar import CodexAppServerSidecar


StopLockCandidates = Callable[[], list[str]]
ArchiveThreadOnce = Callable[[str], None]
SleepFunc = Callable[[float], None]


class ArchiveRetryError(RuntimeError):
    pass


def archive_thread_once(thread_id: str) -> None:
    with CodexAppServerSidecar() as client:
        _ = client.archive_thread(thread_id)


def archive_thread_with_lock_retry(
    thread_id: str,
    *,
    kill_codex_on_lock: bool,
    stop_lock_candidates: StopLockCandidates,
    archive_once: ArchiveThreadOnce = archive_thread_once,
    sleep_func: SleepFunc = time.sleep,
) -> None:
    try:
        archive_once(thread_id)
        return
    except (RuntimeError, OSError, TimeoutError) as exc:
        if not kill_codex_on_lock or not file_backup.is_windows_file_lock_error(exc):
            raise

        original_error = str(exc)
        print(f"archive_lock_error: {bridge_formatting.make_console_safe_text(original_error)}")
        print("archive_lock_retry: stopping Codex processes and retrying once")
        for line in stop_lock_candidates():
            print(line)
        sleep_func(2.0)

        try:
            archive_once(thread_id)
        except (RuntimeError, OSError, TimeoutError) as retry_exc:
            raise ArchiveRetryError(
                "thread/archive failed after Codex process stop retry.\n"
                + f"original_error: {original_error}\n"
                + f"retry_error: {retry_exc}"
            ) from retry_exc
        print("archive_lock_retry: succeeded")

from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout

import codex_desktop_bridge_archive_retry as archive_retry


class ArchiveLockError(OSError):
    pass


class ArchiveOtherError(Exception):
    pass


class ArchiveRetryFailure(RuntimeError):
    pass


def _skip_sleep(seconds: float) -> None:
    _ = seconds


def _no_stop_candidates() -> list[str]:
    return []


class DesktopBridgeArchiveRetryTests(unittest.TestCase):
    def test_archive_thread_with_lock_retry_stops_candidates_and_retries(self) -> None:
        attempts: list[str] = []
        output = io.StringIO()

        def archive_once(thread_id: str) -> None:
            attempts.append(thread_id)
            if len(attempts) == 1:
                raise ArchiveLockError("OS error 32: used by another process")

        with redirect_stdout(output):
            archive_retry.archive_thread_with_lock_retry(
                "thread-1",
                kill_codex_on_lock=True,
                stop_lock_candidates=lambda: ["stopped codex.exe"],
                archive_once=archive_once,
                sleep_func=_skip_sleep,
            )

        self.assertEqual(attempts, ["thread-1", "thread-1"])
        text = output.getvalue()
        self.assertIn("archive_lock_error: OS error 32", text)
        self.assertIn("stopped codex.exe", text)
        self.assertIn("archive_lock_retry: succeeded", text)

    def test_archive_thread_with_lock_retry_propagates_non_lock_failure(self) -> None:
        def archive_once(thread_id: str) -> None:
            _ = thread_id
            raise ArchiveOtherError("not a lock")

        with self.assertRaisesRegex(ArchiveOtherError, "not a lock"):
            archive_retry.archive_thread_with_lock_retry(
                "thread-1",
                kill_codex_on_lock=True,
                stop_lock_candidates=_no_stop_candidates,
                archive_once=archive_once,
            )

    def test_archive_thread_with_lock_retry_wraps_failed_retry(self) -> None:
        attempts: list[str] = []

        def archive_once(thread_id: str) -> None:
            attempts.append(thread_id)
            if len(attempts) == 1:
                raise ArchiveLockError("OS error 32: used by another process")
            raise ArchiveRetryFailure("still locked")

        with self.assertRaisesRegex(
            archive_retry.ArchiveRetryError,
            "thread/archive failed after Codex process stop retry",
        ) as raised:
            archive_retry.archive_thread_with_lock_retry(
                "thread-1",
                kill_codex_on_lock=True,
                stop_lock_candidates=_no_stop_candidates,
                archive_once=archive_once,
                sleep_func=_skip_sleep,
            )

        self.assertIsInstance(raised.exception.__cause__, ArchiveRetryFailure)
        self.assertEqual(attempts, ["thread-1", "thread-1"])


if __name__ == "__main__":
    _ = unittest.main()

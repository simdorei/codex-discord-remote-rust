from __future__ import annotations

import tempfile
import unittest
from io import StringIO
from pathlib import Path
from typing import IO
from unittest import mock

from codex_app_server_stderr_log import (
    AppServerStderrRecorder,
    RotatingAppServerStderrLog,
)


class RotatingAppServerStderrLogTests(unittest.TestCase):
    def test_rotation_keeps_current_file_and_two_backups(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            path = Path(temp_dir) / "app-server.log"
            sink = RotatingAppServerStderrLog(path, max_bytes=5, file_count=3)

            sink.write("1111\n")
            sink.write("2222\n")
            sink.write("3333\n")
            sink.write("4444\n")
            sink.close()

            self.assertEqual(path.read_text(encoding="utf-8"), "4444\n")
            self.assertEqual(
                path.with_name("app-server.log.1").read_text(encoding="utf-8"),
                "3333\n",
            )
            self.assertEqual(
                path.with_name("app-server.log.2").read_text(encoding="utf-8"),
                "2222\n",
            )
            self.assertFalse(path.with_name("app-server.log.3").exists())

    def test_rotation_closes_the_current_handle_before_replacing_on_windows(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            path = Path(temp_dir) / "app-server.log"
            sink = RotatingAppServerStderrLog(path, max_bytes=3, file_count=3)

            sink.write("one")
            sink.write("two")
            sink.close()

            self.assertEqual(path.read_text(encoding="utf-8"), "two")
            self.assertEqual(
                path.with_name("app-server.log.1").read_text(encoding="utf-8"),
                "one",
            )

    def test_recorder_drains_process_stderr_and_closes_the_sink(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            path = Path(temp_dir) / "app-server.log"
            recorder = AppServerStderrRecorder(
                _StderrProcess(StringIO("first\nsecond\n")),
                path,
                log=lambda _line: None,
            )

            recorder.start()
            recorder.close()

            self.assertEqual(path.read_text(encoding="utf-8"), "first\nsecond\n")

    def test_single_oversized_write_never_leaves_a_file_over_the_limit(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            path = Path(temp_dir) / "app-server.log"
            sink = RotatingAppServerStderrLog(path, max_bytes=5, file_count=3)

            sink.write("123456789012")

            self.assertLessEqual(path.stat().st_size, 5)
            self.assertLessEqual(path.with_name("app-server.log.1").stat().st_size, 5)
            self.assertLessEqual(path.with_name("app-server.log.2").stat().st_size, 5)

    def test_rotation_preserves_utf8_character_boundaries(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            path = Path(temp_dir) / "app-server.log"
            sink = RotatingAppServerStderrLog(path, max_bytes=5, file_count=3)

            sink.write("😀😀")

            self.assertEqual(path.read_text(encoding="utf-8"), "😀")
            self.assertEqual(
                path.with_name("app-server.log.1").read_text(encoding="utf-8"),
                "😀",
            )

    def test_write_failure_is_logged_without_stopping_stderr_drain(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            logs: list[str] = []
            recorder = AppServerStderrRecorder(
                _StderrProcess(StringIO("first\nsecond\n")),
                Path(temp_dir) / "app-server.log",
                log=logs.append,
            )

            with mock.patch.object(
                RotatingAppServerStderrLog,
                "write",
                side_effect=OSError("disk full"),
            ) as write:
                recorder.start()
                recorder.close()

            self.assertEqual(write.call_count, 2)
            self.assertEqual(len(logs), 2)
            self.assertTrue(
                all("app_server_stderr_log_write_failed" in line for line in logs)
            )


class _StderrProcess:
    def __init__(self, stderr: IO[str] | None) -> None:
        self.stderr = stderr


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from contextlib import redirect_stderr
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import codex_desktop_bridge as bridge_mod
import codex_discord_logging as logging_mod


class DiscordLoggingTests(unittest.TestCase):
    def test_log_line_writes_timestamped_line_and_rotates(self) -> None:
        rotate_calls: list[tuple[Path, int]] = []

        def rotate(log_path: Path, *, incoming_bytes: int) -> None:
            rotate_calls.append((log_path, incoming_bytes))

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord.log"
            with (
                patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                patch.object(bridge_mod, "rotate_single_backup_file", rotate),
            ):
                logging_mod.log_line("hello")

            content = log_path.read_text(encoding="utf-8")

        self.assertIn("] hello\n", content)
        self.assertEqual(len(rotate_calls), 1)
        self.assertEqual(rotate_calls[0][0], log_path)
        self.assertGreater(rotate_calls[0][1], 0)

    def test_log_line_reports_oserror_to_stderr(self) -> None:
        def rotate(_log_path: Path, *, incoming_bytes: int) -> None:
            _ = incoming_bytes
            raise OSError("disk full")

        stderr = io.StringIO()
        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord.log"
            with (
                patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                patch.object(bridge_mod, "rotate_single_backup_file", rotate),
                redirect_stderr(stderr),
            ):
                logging_mod.log_line("hello")

        output = stderr.getvalue()
        self.assertIn(f"discord_log_write_failed path={log_path}", output)
        self.assertIn("error_type=OSError", output)
        self.assertIn("error=disk full", output)


if __name__ == "__main__":
    _ = unittest.main()

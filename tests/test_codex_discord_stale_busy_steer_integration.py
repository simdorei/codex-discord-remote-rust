from __future__ import annotations

from pathlib import Path
from typing import override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_discord_stale_busy_steer as stale_busy_steer


class StaleLookupUnavailableError(RuntimeError):
    pass


class BadStaleDependencyError(TypeError):
    pass


class DiscordStaleBusySteerIntegrationTests(unittest.TestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "discord-smoke.log")

    @override
    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    def test_get_stale_busy_steer_block_info_passes_through_info(self) -> None:
        expected = ("thread-1", "Thread One", 99.5)

        with mock.patch.object(stale_busy_steer, "get_stale_busy_steer_block_info", return_value=expected):
            result = bot.get_stale_busy_steer_block_info("thread-1")

        self.assertEqual(result, expected)

    def test_get_stale_busy_steer_block_info_runtime_failure_logs_and_returns_none(self) -> None:
        with mock.patch.object(
            stale_busy_steer,
            "get_stale_busy_steer_block_info",
            side_effect=StaleLookupUnavailableError("stale lookup unavailable"),
        ):
            result = bot.get_stale_busy_steer_block_info("thread-1")

        self.assertIsNone(result)
        self.assertIn(
            "stale_busy_steer_check_unavailable target=thread-1 error=stale lookup unavailable",
            self._log_text(),
        )

    def test_get_stale_busy_steer_block_info_type_error_is_not_unavailable(self) -> None:
        with mock.patch.object(
            stale_busy_steer,
            "get_stale_busy_steer_block_info",
            side_effect=BadStaleDependencyError("bad stale dependency"),
        ):
            with self.assertRaisesRegex(TypeError, "bad stale dependency"):
                _ = bot.get_stale_busy_steer_block_info("thread-1")

        self.assertNotIn("stale_busy_steer_check_unavailable", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()

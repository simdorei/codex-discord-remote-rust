from __future__ import annotations

from pathlib import Path
from unittest import mock
import os
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot


class ThreadUnavailableError(RuntimeError):
    pass


class DiscordBotStateIntegrationTests(unittest.TestCase):
    _old_discord_log_path: str | None = None
    _old_mirror_db_path: Path = Path()
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        root = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = root / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(root / "bot-state.log")
        bot.init_mirror_db()

    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    def test_log_path_override_writes_to_temp_file(self) -> None:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])

        bot.log_line("isolated_smoke_log")

        self.assertEqual(bot.get_log_path(), log_path)
        self.assertTrue(log_path.exists())
        self.assertIn("isolated_smoke_log", log_path.read_text(encoding="utf-8"))

    def test_session_mirror_event_has_reflects_committed_claim(self) -> None:
        self.assertFalse(bot.has_session_mirror_event("digest-1", "thread-1"))
        self.assertTrue(bot.claim_session_mirror_event("digest-1", "thread-1"))
        self.assertTrue(bot.has_session_mirror_event("digest-1", "thread-1"))
        self.assertFalse(bot.claim_session_mirror_event("digest-1", "thread-1"))

    def test_recent_codex_app_user_prompt_returns_false_when_thread_unavailable(self) -> None:
        with mock.patch.object(
            bridge,
            "choose_thread",
            side_effect=ThreadUnavailableError("thread unavailable"),
        ):
            result = bot.has_recent_codex_app_user_prompt("thread-1", "hello")

        log_text = self._log_text()
        self.assertFalse(result)
        self.assertIn(
            "recent_codex_prompt_dedupe_unavailable target=thread-1 "
            "reason=choose_thread_failed",
            log_text,
        )
        self.assertIn("ThreadUnavailableError: thread unavailable", log_text)


if __name__ == "__main__":
    _ = unittest.main()

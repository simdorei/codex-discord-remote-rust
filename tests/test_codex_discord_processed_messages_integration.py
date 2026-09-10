from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock
import os
import sqlite3
import tempfile
import unittest

import codex_discord_bot as bot


def _message(message_id: int) -> bot.DiscordMessageIdInput:
    return cast(bot.DiscordMessageIdInput, cast(object, SimpleNamespace(id=message_id)))


class _SlottedOwner:
    __slots__ = ()


class DiscordProcessedMessagesIntegrationTests(unittest.TestCase):
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
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(root / "processed-messages.log")
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

    def test_discord_message_mark_persists_across_restart_after_processing(
        self,
    ) -> None:
        first_owner = SimpleNamespace(_processed_message_ids={})
        restarted_owner = SimpleNamespace(_processed_message_ids={})
        message = _message(123)
        other_message = _message(124)

        self.assertTrue(bot.claim_discord_message(first_owner, message))
        bot.mark_discord_message_processed(first_owner, message)
        self.assertFalse(bot.claim_discord_message(restarted_owner, message))
        self.assertTrue(bot.claim_discord_message(restarted_owner, other_message))

    def test_discord_message_claim_without_mark_still_blocks_restart_duplicate(
        self,
    ) -> None:
        first_owner = SimpleNamespace(_processed_message_ids={})
        restarted_owner = SimpleNamespace(_processed_message_ids={})
        message = _message(123)

        self.assertTrue(bot.claim_discord_message(first_owner, message))
        self.assertFalse(bot.claim_discord_message(restarted_owner, message))

    def test_discord_message_claim_persists_when_seen_map_unsettable(self) -> None:
        first_owner = _SlottedOwner()
        restarted_owner = _SlottedOwner()
        message = _message(123)

        self.assertTrue(bot.claim_discord_message(first_owner, message))
        self.assertFalse(bot.claim_discord_message(first_owner, message))
        self.assertFalse(bot.claim_discord_message(restarted_owner, message))

    def test_processed_message_persistence_failures_log_and_fail_closed(self) -> None:
        owner = _SlottedOwner()
        message = _message(123)

        with mock.patch.object(
            bot.discord_store,
            "claim_persistent_discord_message_id",
            side_effect=sqlite3.OperationalError("locked"),
        ):
            self.assertFalse(bot.claim_discord_message(owner, message))
        with mock.patch.object(
            bot.discord_store,
            "mark_processed_discord_message_id",
            side_effect=OSError("readonly"),
        ):
            bot.mark_discord_message_processed(owner, message)

        log_text = self._log_text()
        self.assertIn(
            "processed_message_persist_failed message=123 error_type=OperationalError",
            log_text,
        )
        self.assertIn(
            "processed_message_mark_failed message=123 error_type=OSError", log_text
        )


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast, override
import os
import sqlite3
import tempfile
import time
import unittest

import codex_discord_bot as bot


class FakeChannel:
    def __init__(self, channel_id: int = 222) -> None:
        self.id: int = channel_id


class FakeComponentChild:
    def __init__(self, custom_id: str) -> None:
        self.custom_id: str = custom_id


class FakeComponentRow:
    def __init__(self, custom_id: str) -> None:
        self.children: list[FakeComponentChild] = [FakeComponentChild(custom_id)]


class FakeComponentMessage:
    def __init__(self, message_id: int, custom_id: str) -> None:
        self.id: int = message_id
        self.channel: FakeChannel = FakeChannel()
        self.components: list[FakeComponentRow] = [FakeComponentRow(custom_id)]
        self.edited_views: list[None] = []

    async def edit(self, view: None = None) -> None:
        self.edited_views.append(view)


class ClearStaleBusyChoiceComponents(Protocol):
    def __call__(self, message: FakeComponentMessage) -> Awaitable[bool]: ...


def _clear_stale_busy_choice_components() -> ClearStaleBusyChoiceComponents:
    return cast(ClearStaleBusyChoiceComponents, bot.clear_stale_busy_choice_message_components)


class DiscordBusyCleanupIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_mirror_db_path: Path | None = None
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        temp_path = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = temp_path / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(temp_path / "discord-smoke.log")
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        if self._old_mirror_db_path is not None:
            bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def test_cleanup_expired_busy_choices_returns_deleted_count(self) -> None:
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.executemany(
                """
                INSERT INTO busy_choices (
                    choice_id, owner_user_id, channel_id, target_thread_id, prompt,
                    allow_steer, created_at, expires_at, claimed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                [
                    ("a" * 24, 1, 222, "thread-1", "expired", 1, 1.0, 2.0, None),
                    ("b" * 24, 1, 222, "thread-1", "claimed", 1, 1.0, 20.0, 3.0),
                    ("c" * 24, 1, 222, "thread-1", "active", 1, 1.0, 20.0, None),
                ],
            )

        deleted = bot.cleanup_expired_busy_choices(now=10.0)
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            remaining = conn.execute("SELECT choice_id FROM busy_choices").fetchall()

        self.assertEqual(deleted, 2)
        self.assertEqual(remaining, [("c" * 24,)])

    def test_cleanup_expired_persistent_component_claims_returns_deleted_count(self) -> None:
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                """
                INSERT INTO persistent_component_claims (claim_key, created_at, expires_at)
                VALUES ('expired-a', 1, 2), ('expired-b', 1, 3), ('live', 1, 20)
                """
            )

        deleted = bot.cleanup_expired_persistent_component_claims(now=10.0)
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            remaining = conn.execute("SELECT claim_key FROM persistent_component_claims").fetchall()

        self.assertEqual(deleted, 2)
        self.assertEqual(remaining, [("live",)])

    async def test_clear_stale_busy_choice_message_components_removes_missing_record(self) -> None:
        message = FakeComponentMessage(123, "codex_busy:0123456789abcdef01234567:steer")

        cleared = await _clear_stale_busy_choice_components()(message)

        self.assertTrue(cleared)
        self.assertEqual(message.edited_views, [None])

    async def test_clear_stale_busy_choice_message_components_keeps_active_record(self) -> None:
        choice_id = "0123456789abcdef01234567"
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                """
                INSERT INTO busy_choices (
                    choice_id, owner_user_id, channel_id, target_thread_id, prompt,
                    allow_steer, created_at, expires_at, claimed_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)
                """,
                (choice_id, 1, 222, "thread-1", "active", 1, 1.0, time.time() + 60.0),
            )
        message = FakeComponentMessage(124, f"codex_busy:{choice_id}:steer")

        cleared = await _clear_stale_busy_choice_components()(message)

        self.assertFalse(cleared)
        self.assertEqual(message.edited_views, [])


if __name__ == "__main__":
    _ = unittest.main()

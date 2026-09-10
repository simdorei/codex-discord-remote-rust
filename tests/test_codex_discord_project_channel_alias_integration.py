# pyright: reportAssignmentType=false, reportPrivateLocalImportUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

import os
import sqlite3
from pathlib import Path
import tempfile
from types import SimpleNamespace
from typing import cast, final, override
import unittest

import codex_discord_bot as bot


class DiscordProjectChannelAliasIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_db_path: Path = Path()
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_db_path = bot.MIRROR_DB_PATH
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(
            Path(temp_dir.name) / "discord-smoke.log"
        )
        bot.MIRROR_DB_PATH = Path(temp_dir.name) / "mirror.sqlite"
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    async def test_merges_normalized_project_key_alias(self) -> None:
        original_text_channel = bot.discord.TextChannel

        @final
        class FakeTextChannel:
            def __init__(self) -> None:
                self.id = 111
                self.name = "codex-taxlab"
                self.topic = "Codex project mirror: taxlab"
                self.category_id = 999

        @final
        class FakeGuild:
            def __init__(self, channel: FakeTextChannel) -> None:
                self.text_channels = [channel]

            def get_channel(self, channel_id: int) -> FakeTextChannel | None:
                return self.text_channels[0] if channel_id == 111 else None

            async def fetch_channel(self, _channel_id: int) -> FakeTextChannel:
                raise AssertionError("cached alias channel should be used")

        try:
            bot.discord.TextChannel = FakeTextChannel
            alias_key = r"\\?\C:\taxlab"
            canonical_key = bot.normalize_project_key(r"C:\taxlab")
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_projects ("
                    + "project_key, project_name, discord_channel_id, updated_at"
                    + ") VALUES (?, ?, ?, ?)",
                    (alias_key, "taxlab", 111, 1.0),
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads ("
                    + "codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at"
                    + ") VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-1", alias_key, "title", 111, 222, 1.0),
                )

            existing = FakeTextChannel()
            channel = cast(
                FakeTextChannel,
                await bot.get_or_create_project_channel(
                    FakeGuild(existing),
                    SimpleNamespace(id=999),
                    r"C:\taxlab",
                    "taxlab",
                ),
            )

            self.assertIs(channel, existing)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                project_rows = cast(
                    list[tuple[str, int]],
                    conn.execute(
                        "SELECT project_key, discord_channel_id FROM mirror_projects"
                    ).fetchall(),
                )
                thread_row = cast(
                    tuple[str] | None,
                    conn.execute(
                        "SELECT project_key FROM mirror_threads WHERE codex_thread_id = ?",
                        ("thread-1",),
                    ).fetchone(),
                )
            self.assertEqual(project_rows, [(canonical_key, 111)])
            self.assertEqual(thread_row, (canonical_key,))
        finally:
            bot.discord.TextChannel = original_text_channel


if __name__ == "__main__":
    _ = unittest.main()

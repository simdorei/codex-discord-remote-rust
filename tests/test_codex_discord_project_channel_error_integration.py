# pyright: reportUnknownMemberType=false
from __future__ import annotations

import os
import sqlite3
from pathlib import Path
import tempfile
from types import SimpleNamespace
from typing import cast, final, override
import unittest

import codex_discord_bot as bot


class FetchChannelError(RuntimeError):
    pass


class DiscordProjectChannelErrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_surfaces_missing_cached_project_channel(self) -> None:
        class FakeGuild:
            text_channels: list[str] = []

            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> None:
                raise FetchChannelError("boom")

            async def create_text_channel(
                self,
                *_unused_args: str,
                **_unused_kwargs: str | int,
            ) -> None:
                raise AssertionError("missing cached channel should not create a replacement")

        canonical_key = bot.normalize_project_key(r"C:\taxlab")
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                "INSERT INTO mirror_projects ("
                + "project_key, project_name, discord_channel_id, updated_at"
                + ") VALUES (?, ?, ?, ?)",
                (canonical_key, "taxlab", 111, 1.0),
            )

        with self.assertRaisesRegex(
            RuntimeError,
            r"Stored mirror project channel 111 .*FetchChannelError: boom",
        ):
            await bot.get_or_create_project_channel(
                FakeGuild(),
                SimpleNamespace(id=999),
                r"C:\taxlab",
                "taxlab",
            )

    async def test_replaces_stored_project_channel_deleted_from_discord(self) -> None:
        original_not_found = bot.discord.NotFound
        original_text_channel = bot.discord.TextChannel

        @final
        class FakeNotFound(RuntimeError):
            pass

        @final
        class FakeTextChannel:
            def __init__(self, channel_id: int, name: str, topic: str) -> None:
                self.id = channel_id
                self.name = name
                self.topic = topic
                self.category_id = 999

        @final
        class FakeGuild:
            text_channels: list[FakeTextChannel] = []

            def __init__(self) -> None:
                self.created: list[FakeTextChannel] = []

            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> None:
                raise FakeNotFound("404 Unknown Channel")

            async def create_text_channel(
                self,
                name: str,
                **kwargs: object,
            ) -> FakeTextChannel:
                channel = FakeTextChannel(222, name, str(kwargs.get("topic") or ""))
                self.created.append(channel)
                return channel

        canonical_key = bot.normalize_project_key(r"C:\taxlab")
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                "INSERT INTO mirror_projects ("
                + "project_key, project_name, discord_channel_id, updated_at"
                + ") VALUES (?, ?, ?, ?)",
                (canonical_key, "taxlab", 111, 1.0),
            )

        try:
            bot.discord.NotFound = FakeNotFound
            bot.discord.TextChannel = FakeTextChannel
            guild = FakeGuild()

            channel = cast(
                FakeTextChannel,
                await bot.get_or_create_project_channel(
                    guild,
                    SimpleNamespace(id=999),
                    r"C:\taxlab",
                    "taxlab",
                ),
            )

            self.assertEqual(channel.id, 222)
            self.assertEqual(len(guild.created), 1)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[int] | None,
                    conn.execute(
                        "SELECT discord_channel_id FROM mirror_projects WHERE project_key = ?",
                        (canonical_key,),
                    ).fetchone(),
                )
            self.assertEqual(row, (222,))
        finally:
            bot.discord.NotFound = original_not_found
            bot.discord.TextChannel = original_text_channel


if __name__ == "__main__":
    _ = unittest.main()

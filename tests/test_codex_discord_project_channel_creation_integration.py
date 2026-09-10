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


class MissingDbChannelError(RuntimeError):
    pass


class DiscordProjectChannelCreationIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_reuses_existing_mirror_channel(self) -> None:
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

            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> FakeTextChannel:
                raise MissingDbChannelError("missing db channel should not matter")

            async def create_text_channel(
                self,
                *_unused_args: str,
                **_unused_kwargs: str | int,
            ) -> FakeTextChannel:
                raise AssertionError("existing mirror project channel should be reused")

        try:
            bot.discord.TextChannel = FakeTextChannel
            existing = FakeTextChannel()
            category = SimpleNamespace(id=999)

            channel = cast(
                FakeTextChannel,
                await bot.get_or_create_project_channel(
                    FakeGuild(existing),
                    category,
                    r"c:\taxlab",
                    "taxlab",
                ),
            )

            self.assertIs(channel, existing)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[int] | None,
                    conn.execute(
                        "SELECT discord_channel_id FROM mirror_projects WHERE project_key = ?",
                        (r"c:\taxlab",),
                    ).fetchone(),
                )
            self.assertEqual(row, (111,))
        finally:
            bot.discord.TextChannel = original_text_channel

    async def test_does_not_reuse_non_mirror_name_match(self) -> None:
        original_text_channel = bot.discord.TextChannel

        @final
        class FakeTextChannel:
            def __init__(self, channel_id: int, name: str, topic: str = "") -> None:
                self.id = channel_id
                self.name = name
                self.topic = topic
                self.category_id = 999

        @final
        class FakeGuild:
            def __init__(self, channel: FakeTextChannel) -> None:
                self.text_channels = [channel]
                self.created: list[tuple[str, dict[str, str | int]]] = []

            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> FakeTextChannel:
                raise MissingDbChannelError("missing db channel should not matter")

            async def create_text_channel(
                self,
                name: str,
                **kwargs: str | int,
            ) -> FakeTextChannel:
                self.created.append((name, kwargs))
                channel = FakeTextChannel(222, name, str(kwargs.get("topic") or ""))
                self.text_channels.append(channel)
                return channel

        try:
            bot.discord.TextChannel = FakeTextChannel
            existing = FakeTextChannel(111, "codex-taxlab", "")
            guild = FakeGuild(existing)
            category = SimpleNamespace(id=999)

            channel = cast(
                FakeTextChannel,
                await bot.get_or_create_project_channel(
                    guild,
                    category,
                    r"c:\taxlab",
                    "taxlab",
                ),
            )

            self.assertIsNot(channel, existing)
            self.assertEqual(channel.id, 222)
            self.assertEqual(len(guild.created), 1)
            self.assertTrue(guild.created[0][0].startswith("codex-taxlab-"))
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[int] | None,
                    conn.execute(
                        "SELECT discord_channel_id FROM mirror_projects WHERE project_key = ?",
                        (r"c:\taxlab",),
                    ).fetchone(),
                )
            self.assertEqual(row, (222,))
        finally:
            bot.discord.TextChannel = original_text_channel

    async def test_renames_cached_mirror_channel(self) -> None:
        original_text_channel = bot.discord.TextChannel

        @final
        class FakeTextChannel:
            def __init__(self) -> None:
                self.id = 111
                self.name = "old-name"
                self.topic = "old topic"
                self.category_id = 999
                self.edits: list[dict[str, str]] = []

            async def edit(self, **kwargs: str) -> None:
                self.edits.append(kwargs)
                self.name = str(kwargs.get("name", self.name))
                self.topic = str(kwargs.get("topic", self.topic))

        @final
        class FakeGuild:
            def __init__(self, channel: FakeTextChannel) -> None:
                self.text_channels = [channel]

            def get_channel(self, channel_id: int) -> FakeTextChannel | None:
                return self.text_channels[0] if channel_id == 111 else None

            async def fetch_channel(self, _channel_id: int) -> FakeTextChannel:
                raise AssertionError("cached project channel should be used")

        try:
            bot.discord.TextChannel = FakeTextChannel
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                _ = conn.execute(
                    """
                    INSERT INTO mirror_projects (
                        project_key, project_name, discord_channel_id, updated_at
                    ) VALUES (?, ?, ?, ?)
                    """,
                    (r"c:\taxlab", "old", 111, 1.0),
                )
            existing = FakeTextChannel()

            channel = cast(
                FakeTextChannel,
                await bot.get_or_create_project_channel(
                    FakeGuild(existing),
                    SimpleNamespace(id=999),
                    r"c:\taxlab",
                    "taxlab",
                ),
            )

            self.assertIs(channel, existing)
            self.assertEqual(existing.name, "codex-taxlab")
            self.assertEqual(existing.topic, "Codex project mirror: taxlab")
            self.assertEqual(len(existing.edits), 1)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[str, int] | None,
                    conn.execute(
                        """
                        SELECT project_name, discord_channel_id
                        FROM mirror_projects
                        WHERE project_key = ?
                        """,
                        (r"c:\taxlab",),
                    ).fetchone(),
                )
            self.assertEqual(row, ("taxlab", 111))
        finally:
            bot.discord.TextChannel = original_text_channel


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

import unittest
from typing import cast

import discord

import codex_discord_mirror_stale as mirror_stale


class MirrorStaleCleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_delete_stale_discord_threads_deletes_cached_thread(self) -> None:
        original_thread = discord.Thread

        class FakeThread:
            def __init__(self) -> None:
                self.deleted_reasons: list[str] = []

            async def delete(self, *, reason: str) -> None:
                self.deleted_reasons.append(reason)

        class FakeGuild:
            def __init__(self, thread: FakeThread) -> None:
                self.thread: FakeThread = thread
                self.fetch_calls: list[int] = []

            def get_thread(self, thread_id: int) -> FakeThread | None:
                if thread_id == 111:
                    return self.thread
                return None

            async def fetch_channel(self, channel_id: int) -> FakeThread:
                self.fetch_calls.append(channel_id)
                return self.thread

        try:
            discord.Thread = FakeThread
            thread = FakeThread()
            guild = FakeGuild(thread)
            result = await mirror_stale.delete_stale_discord_threads(
                cast(discord.Guild, cast(object, guild)),
                [("thread-123456", 111, "Title")],
            )
        finally:
            discord.Thread = original_thread

        self.assertEqual(result["deleted"], 1)
        self.assertEqual(result["missing"], 0)
        self.assertEqual(result["failed"], 0)
        self.assertEqual(guild.fetch_calls, [])
        self.assertEqual(len(thread.deleted_reasons), 1)
        self.assertIn("thread-1", thread.deleted_reasons[0])

    async def test_delete_stale_project_channels_skips_non_mirror_text_channel(self) -> None:
        original_text_channel = discord.TextChannel

        class FakeTextChannel:
            def __init__(self) -> None:
                self.category_id: int = 123
                self.topic: str = "general"
                self.deleted: bool = False

            async def delete(self, *, reason: str) -> None:
                _ = reason
                self.deleted = True

        class FakeGuild:
            def __init__(self, channel: FakeTextChannel) -> None:
                self.channel: FakeTextChannel = channel

            def get_channel(self, channel_id: int) -> FakeTextChannel | None:
                if channel_id == 222:
                    return self.channel
                return None

            async def fetch_channel(self, _channel_id: int) -> FakeTextChannel:
                return self.channel

        class FakeCategory:
            id: int = 999

        try:
            discord.TextChannel = FakeTextChannel
            channel = FakeTextChannel()
            result = await mirror_stale.delete_stale_project_channels(
                cast(discord.Guild, cast(object, FakeGuild(channel))),
                cast(discord.CategoryChannel, cast(object, FakeCategory())),
                [("stale-project", "stale", 222)],
            )
        finally:
            discord.TextChannel = original_text_channel

        self.assertEqual(result["deleted"], 0)
        self.assertEqual(result["skipped"], 1)
        self.assertFalse(channel.deleted)

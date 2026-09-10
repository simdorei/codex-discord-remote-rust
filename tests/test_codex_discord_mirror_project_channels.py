from __future__ import annotations

import unittest
from typing import cast

import discord

import codex_discord_mirror_project_channels as mirror_project_channels


class MirrorProjectChannelHelperTests(unittest.IsolatedAsyncioTestCase):
    async def test_orphan_cleanup_project_channels_prefers_cache_and_fetches_missing(
        self,
    ) -> None:
        original_text_channel = discord.TextChannel

        class FakeTextChannel:
            def __init__(self, channel_id: int) -> None:
                self.id: int = channel_id

        class FakeGuild:
            def __init__(self) -> None:
                self.cached: FakeTextChannel = FakeTextChannel(111)
                self.fetched: FakeTextChannel = FakeTextChannel(222)
                self.fetch_calls: list[int] = []

            def get_channel(self, channel_id: int) -> FakeTextChannel | None:
                if channel_id == self.cached.id:
                    return self.cached
                return None

            async def fetch_channel(self, channel_id: int) -> FakeTextChannel:
                self.fetch_calls.append(channel_id)
                return self.fetched

        try:
            discord.TextChannel = FakeTextChannel
            guild = FakeGuild()
            channels = await mirror_project_channels.resolve_orphan_cleanup_project_channels(
                cast(discord.Guild, cast(object, guild)),
                [111, 222],
                fetch_failure_types=(RuntimeError,),
            )
        finally:
            discord.TextChannel = original_text_channel

        self.assertEqual(channels, [guild.cached, guild.fetched])
        self.assertEqual(guild.fetch_calls, [222])

    async def test_orphan_cleanup_project_channels_ignores_delivery_fetch_failures(
        self,
    ) -> None:
        class FakeFetchError(RuntimeError):
            pass

        class FakeGuild:
            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> None:
                raise FakeFetchError("gone")

        channels = await mirror_project_channels.resolve_orphan_cleanup_project_channels(
            cast(discord.Guild, cast(object, FakeGuild())),
            [333],
            fetch_failure_types=(FakeFetchError,),
        )

        self.assertEqual(channels, [])

    async def test_orphan_cleanup_project_channels_propagates_unexpected_fetch_bug(
        self,
    ) -> None:
        class FakeUnexpectedFetchBug(TypeError):
            pass

        class FakeGuild:
            def get_channel(self, _channel_id: int) -> None:
                return None

            async def fetch_channel(self, _channel_id: int) -> None:
                raise FakeUnexpectedFetchBug("project channel fetch bug")

        with self.assertRaisesRegex(TypeError, "project channel fetch bug"):
            _ = await mirror_project_channels.resolve_orphan_cleanup_project_channels(
                cast(discord.Guild, cast(object, FakeGuild())),
                [333],
                fetch_failure_types=(RuntimeError,),
            )

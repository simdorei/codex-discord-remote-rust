from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_channel_cache as channel_cache


@dataclass(frozen=True, slots=True)
class _FakeChannel:
    name: str


@dataclass(frozen=True, slots=True)
class _FakeGuild:
    thread: _FakeChannel | None = None
    channel: _FakeChannel | None = None

    def get_thread(self, _channel_id: int) -> _FakeChannel | None:
        return self.thread

    def get_channel(self, _channel_id: int) -> _FakeChannel | None:
        return self.channel


@dataclass(frozen=True, slots=True)
class _FakeClient:
    channel: _FakeChannel | None = None
    guilds: tuple[_FakeGuild, ...] = ()

    def get_channel(self, _channel_id: int) -> _FakeChannel | None:
        return self.channel


class DiscordChannelCacheTests(unittest.TestCase):
    def test_uses_client_channel_cache_first(self) -> None:
        cached_channel = _FakeChannel("client")
        guild_thread = _FakeChannel("thread")

        channel, source = channel_cache.get_cached_channel_or_thread(
            _FakeClient(
                channel=cached_channel,
                guilds=(_FakeGuild(thread=guild_thread),),
            ),
            123,
        )

        self.assertIs(channel, cached_channel)
        self.assertEqual(source, "client_channel_cache")

    def test_uses_guild_thread_before_guild_channel(self) -> None:
        guild_thread = _FakeChannel("thread")
        guild_channel = _FakeChannel("channel")

        channel, source = channel_cache.get_cached_channel_or_thread(
            _FakeClient(guilds=(_FakeGuild(thread=guild_thread, channel=guild_channel),)),
            123,
        )

        self.assertIs(channel, guild_thread)
        self.assertEqual(source, "guild_thread_cache")

    def test_uses_guild_channel_when_thread_missing(self) -> None:
        guild_channel = _FakeChannel("channel")

        channel, source = channel_cache.get_cached_channel_or_thread(
            _FakeClient(guilds=(_FakeGuild(channel=guild_channel),)),
            123,
        )

        self.assertIs(channel, guild_channel)
        self.assertEqual(source, "guild_channel_cache")

    def test_returns_missing_source_when_not_cached(self) -> None:
        channel, source = channel_cache.get_cached_channel_or_thread(_FakeClient(), 123)

        self.assertIsNone(channel)
        self.assertEqual(source, "-")


if __name__ == "__main__":
    _ = unittest.main()

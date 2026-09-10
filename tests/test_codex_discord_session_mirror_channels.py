from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_session_mirror_channels as channels


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int


class FetchFailure(Exception):
    pass


class SessionMirrorChannelResolverTests(unittest.IsolatedAsyncioTestCase):
    async def test_cached_messageable_channel_returns_without_fetch(self) -> None:
        logs: list[str] = []
        fetched: list[int] = []
        cached = FakeChannel(channel_id=123)

        async def fetch_channel(channel_id: int) -> FakeChannel:
            fetched.append(channel_id)
            raise FetchFailure("fetch should not run")

        deps: channels.SessionMirrorChannelResolveDeps[FakeChannel] = (
            channels.SessionMirrorChannelResolveDeps(
                get_cached_channel_or_thread=lambda channel_id: (cached, "cache"),
                fetch_channel=fetch_channel,
                fetch_failure_types=(FetchFailure,),
                is_messageable=lambda channel: True,
                log=logs.append,
            )
        )
        resolved = await channels.resolve_session_mirror_channel(
            123,
            deps=deps,
        )

        self.assertIs(resolved, cached)
        self.assertEqual(fetched, [])
        self.assertEqual(logs, [])

    async def test_fetch_messageable_channel_returns_fetched_channel(self) -> None:
        logs: list[str] = []
        fetched = FakeChannel(channel_id=123)

        async def fetch_channel(channel_id: int) -> FakeChannel:
            self.assertEqual(channel_id, 123)
            return fetched

        deps: channels.SessionMirrorChannelResolveDeps[FakeChannel] = (
            channels.SessionMirrorChannelResolveDeps(
                get_cached_channel_or_thread=lambda channel_id: (None, "cache"),
                fetch_channel=fetch_channel,
                fetch_failure_types=(FetchFailure,),
                is_messageable=lambda channel: True,
                log=logs.append,
            )
        )
        resolved = await channels.resolve_session_mirror_channel(
            123,
            deps=deps,
        )

        self.assertIs(resolved, fetched)
        self.assertEqual(logs, [])

    async def test_fetch_failure_logs_and_returns_none(self) -> None:
        logs: list[str] = []

        async def fetch_channel(channel_id: int) -> FakeChannel:
            self.assertEqual(channel_id, 123)
            raise FetchFailure("missing")

        deps: channels.SessionMirrorChannelResolveDeps[FakeChannel] = (
            channels.SessionMirrorChannelResolveDeps(
                get_cached_channel_or_thread=lambda channel_id: (None, "cache"),
                fetch_channel=fetch_channel,
                fetch_failure_types=(FetchFailure,),
                is_messageable=lambda channel: True,
                log=logs.append,
            )
        )
        resolved = await channels.resolve_session_mirror_channel(
            123,
            deps=deps,
        )

        self.assertIsNone(resolved)
        self.assertEqual(logs, ["session_mirror_channel_failed channel=123 error_type=FetchFailure"])

    async def test_non_messageable_cached_channel_logs_and_returns_none(self) -> None:
        logs: list[str] = []
        cached = FakeChannel(channel_id=123)

        async def fetch_channel(_channel_id: int) -> FakeChannel:
            raise FetchFailure("fetch should not run")

        deps: channels.SessionMirrorChannelResolveDeps[FakeChannel] = (
            channels.SessionMirrorChannelResolveDeps(
                get_cached_channel_or_thread=lambda channel_id: (cached, "cache"),
                fetch_channel=fetch_channel,
                fetch_failure_types=(FetchFailure,),
                is_messageable=lambda channel: False,
                log=logs.append,
            )
        )
        resolved = await channels.resolve_session_mirror_channel(
            123,
            deps=deps,
        )

        self.assertIsNone(resolved)
        self.assertEqual(
            logs,
            ["session_mirror_channel_skipped channel=123 source=cache reason=not_messageable"],
        )

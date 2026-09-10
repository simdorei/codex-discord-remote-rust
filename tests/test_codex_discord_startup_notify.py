from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_bot as bot
import codex_discord_startup_notify as startup_notify


class _FakeMessageable:
    pass


@dataclass(frozen=True, slots=True)
class _FakeChannel(_FakeMessageable):
    channel_id: int


@dataclass(frozen=True, slots=True)
class _FakeClient:
    cached_channel: _FakeChannel | None = None
    fetched_channel: _FakeChannel | None = None
    fetch_error: BaseException | None = None

    def get_channel(self, _channel_id: int) -> _FakeChannel | None:
        return self.cached_channel

    async def fetch_channel(self, _channel_id: int) -> _FakeChannel:
        if self.fetch_error is not None:
            raise self.fetch_error
        if self.fetched_channel is None:
            raise AssertionError("missing fetched channel")
        return self.fetched_channel


class DiscordStartupNotifyTests(unittest.IsolatedAsyncioTestCase):
    def _is_messageable(self, channel: _FakeChannel) -> bool:
        _ = channel
        return True

    def test_startup_notice_is_actionable(self) -> None:
        notice = bot.build_startup_notice()

        self.assertIn("Codex Discord bot online.", notice)
        self.assertIn("restart/startup completed", notice)
        self.assertIn("new Discord messages and slash commands are accepted", notice)
        self.assertIn("`!where` or `/where`", notice)
        self.assertIn("`!help` or `/help`", notice)

    async def test_sends_notice_to_cached_startup_channel(self) -> None:
        sent_channels: list[_FakeChannel] = []
        logs: list[str] = []

        async def send_chunks(channel: _FakeChannel, text: str, *, context: str) -> int:
            _ = text
            self.assertEqual(context, "startup_notify")
            sent_channels.append(channel)
            return 1

        await startup_notify.send_startup_notice_if_enabled(
            _FakeClient(cached_channel=_FakeChannel(123)),
            123,
            notify_enabled=lambda: True,
            is_messageable=self._is_messageable,
            send_chunks=send_chunks,
            build_startup_notice=lambda: "notice",
            log=logs.append,
            delivery_exceptions=(RuntimeError,),
        )

        self.assertEqual([channel.channel_id for channel in sent_channels], [123])
        self.assertIn("startup_notify_sent channel=123", logs)

    async def test_fetch_failure_logs_and_returns(self) -> None:
        logs: list[str] = []

        async def send_chunks(channel: _FakeChannel, text: str, *, context: str) -> int:
            _ = (channel, text)
            raise AssertionError(f"unexpected send: {context}")

        await startup_notify.send_startup_notice_if_enabled(
            _FakeClient(fetch_error=RuntimeError("fetch failed")),
            123,
            notify_enabled=lambda: True,
            is_messageable=self._is_messageable,
            send_chunks=send_chunks,
            build_startup_notice=lambda: "notice",
            log=logs.append,
            delivery_exceptions=(RuntimeError,),
        )

        self.assertTrue(any("startup_channel_fetch_failed" in line for line in logs))


if __name__ == "__main__":
    _ = unittest.main()

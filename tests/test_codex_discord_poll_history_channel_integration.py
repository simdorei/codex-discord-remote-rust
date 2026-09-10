from __future__ import annotations

from collections.abc import AsyncIterator, Awaitable
from pathlib import Path
from typing import Never, Protocol, cast, override
import os
import tempfile
import unittest

import codex_discord_bot as bot
from codex_discord_diagnostics_history import DiscordChannelLike


class HistoryFetchUnavailableError(RuntimeError):
    pass


class HistoryIteratorUnavailableError(RuntimeError):
    pass


class UnexpectedFetchError(AssertionError):
    pass


class FetchChannel(Protocol):
    def __call__(self, channel_id: int) -> Awaitable[DiscordChannelLike | None]: ...


class PollHistoryClient:
    def __init__(self, cached_channel: DiscordChannelLike | None, source: str, fetch_channel: FetchChannel) -> None:
        self._history_poll_primed_channels: set[int] = set()
        self._cached_channel: DiscordChannelLike | None = cached_channel
        self._source: str = source
        self._fetch_channel: FetchChannel = fetch_channel

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[DiscordChannelLike | None, str]:
        _ = channel_id
        return self._cached_channel, self._source

    async def fetch_channel(self, channel_id: int) -> DiscordChannelLike | None:
        return await self._fetch_channel(channel_id)


class RuntimeFailingHistoryChannel:
    id: int = 333

    def history(self, *, limit: int) -> AsyncIterator[Never]:
        _ = limit
        raise HistoryIteratorUnavailableError("history iterator unavailable")


class TypeFailingHistoryChannel:
    id: int = 333

    def history(self, *, limit: int) -> AsyncIterator[Never]:
        _ = limit
        _raise_type_error("bad history iterator dependency")


class PollHistoryChannel(Protocol):
    def __call__(self, client: PollHistoryClient, label: str, channel_id: int) -> Awaitable[None]: ...


def _poll_history_channel() -> PollHistoryChannel:
    return cast(PollHistoryChannel, bot.CodexDiscordBot.poll_history_channel)


def _raise_type_error(message: str) -> Never:
    return (_ for _ in ()).throw(TypeError(message))


async def _unexpected_fetch(channel_id: int) -> DiscordChannelLike | None:
    _ = channel_id
    raise UnexpectedFetchError("fetch not expected")


class DiscordPollHistoryChannelIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "discord-smoke.log")

    @override
    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        return log_path.read_text(encoding="utf-8") if log_path.exists() else ""

    async def test_poll_history_channel_fetch_runtime_failure_logs_and_returns(self) -> None:
        async def fetch_channel(channel_id: int) -> DiscordChannelLike | None:
            _ = channel_id
            raise HistoryFetchUnavailableError("history fetch unavailable")

        client = PollHistoryClient(None, "-", fetch_channel)

        await _poll_history_channel()(client, "allowed", 333)

        log_text = self._log_text()
        self.assertIn("history_poll_channel_failed label=allowed channel=333", log_text)
        self.assertIn("error_type=HistoryFetchUnavailableError", log_text)

    async def test_poll_history_channel_fetch_type_error_is_not_channel_failed(self) -> None:
        async def fetch_channel(channel_id: int) -> DiscordChannelLike | None:
            _ = channel_id
            _raise_type_error("bad history fetch dependency")

        client = PollHistoryClient(None, "-", fetch_channel)

        with self.assertRaisesRegex(TypeError, "bad history fetch dependency"):
            await _poll_history_channel()(client, "allowed", 333)

        self.assertNotIn("history_poll_channel_failed label=allowed channel=333", self._log_text())

    async def test_poll_history_channel_history_runtime_failure_logs_and_returns(self) -> None:
        client = PollHistoryClient(cast(DiscordChannelLike, RuntimeFailingHistoryChannel()), "test_cache", _unexpected_fetch)

        await _poll_history_channel()(client, "allowed", 333)

        log_text = self._log_text()
        self.assertIn("history_poll_channel_failed label=allowed channel=333", log_text)
        self.assertIn("source=test_cache", log_text)
        self.assertIn("error_type=HistoryIteratorUnavailableError", log_text)

    async def test_poll_history_channel_history_type_error_is_not_channel_failed(self) -> None:
        client = PollHistoryClient(cast(DiscordChannelLike, TypeFailingHistoryChannel()), "test_cache", _unexpected_fetch)

        with self.assertRaisesRegex(TypeError, "bad history iterator dependency"):
            await _poll_history_channel()(client, "allowed", 333)

        self.assertNotIn("history_poll_channel_failed label=allowed channel=333", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()

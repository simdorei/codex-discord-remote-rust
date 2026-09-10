from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Never, Protocol, cast
import asyncio  # noqa: ANYIO_OK
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot


class HistoryPollTargetsUnavailableError(RuntimeError):
    pass


class PollHistoryChannel(Protocol):
    def __call__(self, label: str, channel_id: int) -> Awaitable[None]: ...


class HistoryPollClient:
    def __init__(self, poll_history_channel: PollHistoryChannel, *, close_after_checks: int | None = None) -> None:
        self.allowed_channel_ids: set[int] = {333}
        self.startup_channel_id: None = None
        self.history_poll_seconds: float = 0.01
        self._history_poll_last_at: str = "-"
        self.poll_history_channel: PollHistoryChannel = poll_history_channel
        self._closed_checks: int = 0
        self._close_after_checks: int | None = close_after_checks

    def is_closed(self) -> bool:
        self._closed_checks += 1
        return self._close_after_checks is not None and self._closed_checks > self._close_after_checks


class HistoryPollLoop(Protocol):
    def __call__(self, client: HistoryPollClient) -> Awaitable[None]: ...


def _history_poll_loop() -> HistoryPollLoop:
    return cast(HistoryPollLoop, bot.CodexDiscordBot.history_poll_loop)


def _raise_type_error(message: str) -> Never:
    return (_ for _ in ()).throw(TypeError(message))


class DiscordHistoryPollLoopIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_history_poll_loop_continues_after_cycle_error(self) -> None:
        calls: list[str] = []
        sleeps = 0

        def fake_get_targets(
            allowed_channel_ids: set[int],
            startup_channel_id: int | None,
            *,
            limit: int = 50,
        ) -> list[tuple[str, int]]:
            _ = (allowed_channel_ids, startup_channel_id, limit)
            calls.append("targets")
            if len(calls) == 1:
                raise HistoryPollTargetsUnavailableError("temporary db error")
            return [("allowed", 333)]

        async def fake_sleep(seconds: float) -> None:
            _ = seconds
            nonlocal sleeps
            sleeps += 1
            if sleeps >= 2:
                raise asyncio.CancelledError

        async def fake_poll(label: str, channel_id: int) -> None:
            calls.append(f"poll:{label}:{channel_id}")

        client = HistoryPollClient(fake_poll)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot, "get_startup_probe_targets", fake_get_targets),
                mock.patch("codex_discord_bot.asyncio.sleep", fake_sleep),
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                self.assertRaises(asyncio.CancelledError),
            ):
                await _history_poll_loop()(client)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, ["targets", "targets", "poll:allowed:333"])
        self.assertIn("history_poll_cycle_failed", log_text)
        self.assertIn("temporary db error", log_text)

    async def test_history_poll_loop_type_error_is_not_cycle_failed(self) -> None:
        def fake_get_targets(
            allowed_channel_ids: set[int],
            startup_channel_id: int | None,
            *,
            limit: int = 50,
        ) -> list[tuple[str, int]]:
            _ = (allowed_channel_ids, startup_channel_id, limit)
            _raise_type_error("bad history poll target dependency")

        async def fake_sleep(seconds: float) -> None:
            _ = seconds

        async def fake_poll(label: str, channel_id: int) -> None:
            _ = (label, channel_id)

        client = HistoryPollClient(fake_poll, close_after_checks=1)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot, "get_startup_probe_targets", fake_get_targets),
                mock.patch("codex_discord_bot.asyncio.sleep", fake_sleep),
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
                self.assertRaisesRegex(TypeError, "bad history poll target dependency"),
            ):
                await _history_poll_loop()(client)
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

        self.assertNotIn("history_poll_cycle_failed", log_text)


if __name__ == "__main__":
    _ = unittest.main()

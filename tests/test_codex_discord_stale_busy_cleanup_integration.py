from __future__ import annotations

from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock
import os
import tempfile
import unittest

import codex_discord_bot as bot


class FakeMessageable:
    pass


class FakeChannel(FakeMessageable):
    pass


class StaleCleanupFetchUnavailableError(RuntimeError):
    pass


class BadStaleCleanupFetchDependencyError(TypeError):
    pass


class StaleCleanupUnavailableError(RuntimeError):
    pass


class BadStaleCleanupDependencyError(TypeError):
    pass


@contextmanager
def _patched_messageable() -> Generator[None, None, None]:
    original_messageable = bot.discord.abc.Messageable
    try:
        bot.discord.abc.Messageable = FakeMessageable
        yield
    finally:
        bot.discord.abc.Messageable = original_messageable


def _bot_client(value: SimpleNamespace) -> bot.CodexDiscordBot:
    return cast(bot.CodexDiscordBot, cast(object, value))


async def _fail_fetch(message: str, channel_id: int) -> None:
    _ = channel_id
    raise AssertionError(message)


class DiscordStaleBusyCleanupIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "stale-cleanup.log")

    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_cleanup_stale_busy_components_fetch_runtime_failure_logs_and_continues(self) -> None:
        async def fetch_channel(channel_id: int) -> None:
            _ = channel_id
            raise StaleCleanupFetchUnavailableError("stale cleanup fetch unavailable")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            get_cached_channel_or_thread=lambda channel_id: (None, "-"),
            fetch_channel=fetch_channel,
        )

        with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
            await bot.CodexDiscordBot.cleanup_stale_busy_choice_components(_bot_client(client))

        log_text = self._log_text()
        self.assertIn("stale_busy_choice_component_cleanup_skipped label=allowed", log_text)
        self.assertIn(
            "channel=123 reason=fetch_failed error_type=StaleCleanupFetchUnavailableError",
            log_text,
        )

    async def test_cleanup_stale_busy_components_fetch_type_error_is_not_skipped(self) -> None:
        async def fetch_channel(channel_id: int) -> None:
            _ = channel_id
            raise BadStaleCleanupFetchDependencyError("bad stale cleanup fetch dependency")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            get_cached_channel_or_thread=lambda channel_id: (None, "-"),
            fetch_channel=fetch_channel,
        )

        with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
            with self.assertRaisesRegex(
                BadStaleCleanupFetchDependencyError,
                "bad stale cleanup fetch dependency",
            ):
                await bot.CodexDiscordBot.cleanup_stale_busy_choice_components(_bot_client(client))

        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
        self.assertNotIn("stale_busy_choice_component_cleanup_skipped label=allowed", log_text)

    async def test_cleanup_stale_busy_components_runtime_cleanup_failure_logs_and_continues(self) -> None:
        channel = FakeChannel()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached stale cleanup channel should not fetch", channel_id)

        async def cleanup_channel(message_channel) -> int:
            _ = message_channel
            raise StaleCleanupUnavailableError("stale cleanup unavailable")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
        )
        with _patched_messageable():
            with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
                with mock.patch.object(bot, "cleanup_stale_busy_choice_components_in_channel", cleanup_channel):
                    await bot.CodexDiscordBot.cleanup_stale_busy_choice_components(_bot_client(client))

        log_text = self._log_text()
        self.assertIn("stale_busy_choice_component_cleanup_failed label=allowed channel=123", log_text)
        self.assertIn("StaleCleanupUnavailableError: stale cleanup unavailable", log_text)

    async def test_cleanup_stale_busy_components_type_error_is_not_cleanup_failed(self) -> None:
        channel = FakeChannel()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached stale cleanup channel should not fetch", channel_id)

        async def cleanup_channel(message_channel) -> int:
            _ = message_channel
            raise BadStaleCleanupDependencyError("bad stale cleanup dependency")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
        )
        with _patched_messageable():
            with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
                with mock.patch.object(bot, "cleanup_stale_busy_choice_components_in_channel", cleanup_channel):
                    with self.assertRaisesRegex(
                        BadStaleCleanupDependencyError,
                        "bad stale cleanup dependency",
                    ):
                        await bot.CodexDiscordBot.cleanup_stale_busy_choice_components(_bot_client(client))

        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
        self.assertNotIn("stale_busy_choice_component_cleanup_failed label=allowed", log_text)

    async def test_cleanup_stale_busy_components_deleted_count_logs_done(self) -> None:
        channel = FakeChannel()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached stale cleanup channel should not fetch", channel_id)

        async def cleanup_channel(message_channel) -> int:
            self.assertIs(message_channel, channel)
            return 2

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
        )
        with _patched_messageable():
            with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
                with mock.patch.object(bot, "cleanup_stale_busy_choice_components_in_channel", cleanup_channel):
                    await bot.CodexDiscordBot.cleanup_stale_busy_choice_components(_bot_client(client))

        log_text = self._log_text()
        self.assertIn("stale_busy_choice_component_cleanup_deleted label=allowed channel=123 count=2", log_text)
        self.assertIn("stale_busy_choice_component_cleanup_done count=2", log_text)


if __name__ == "__main__":
    _ = unittest.main()

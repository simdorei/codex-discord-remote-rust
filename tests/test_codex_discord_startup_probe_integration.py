from __future__ import annotations

from collections.abc import Generator
from contextlib import contextmanager
from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock
import asyncio  # noqa: ANYIO_OK
import os
import tempfile
import unittest

import codex_discord_bot as bot


class FakeMessageable:
    pass


class ParentChannel(FakeMessageable):
    parent_id = 555


class StartupFetchUnavailableError(RuntimeError):
    pass


class BadStartupFetchDependencyError(TypeError):
    pass


class AllowedCheckUnavailableError(RuntimeError):
    pass


class BadAllowedCheckDependencyError(TypeError):
    pass


class StartupProbeUnavailableError(RuntimeError):
    pass


class BadStartupDiagnosticsDependencyError(TypeError):
    pass


@contextmanager
def _patched_messageable() -> Generator[None, None, None]:
    original_messageable = bot.discord.abc.Messageable
    try:
        bot.discord.abc.Messageable = FakeMessageable
        yield
    finally:
        bot.discord.abc.Messageable = original_messageable


async def _fail_fetch(message: str, channel_id: int) -> None:
    _ = channel_id
    raise AssertionError(message)


def _bot_client(value: SimpleNamespace) -> bot.CodexDiscordBot:
    return cast(bot.CodexDiscordBot, cast(object, value))


class DiscordStartupProbeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "startup-probe.log")

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

    async def test_resolve_session_mirror_channel_uses_cached_messageable_channel(self) -> None:
        channel = FakeMessageable()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached session mirror channel should not fetch", channel_id)

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
        )

        with _patched_messageable():
            resolved = await bot.CodexDiscordBot.resolve_session_mirror_channel(_bot_client(client), 123)

        self.assertIs(resolved, channel)

    async def test_probe_channel_access_logs_allowed_cached_messageable_channel(self) -> None:
        channel = ParentChannel()
        allowed_calls: list[str] = []

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached startup probe channel should not fetch", channel_id)

        def is_allowed_message_channel(message_channel) -> bool:
            allowed_calls.append(type(message_channel).__name__)
            return True

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=is_allowed_message_channel,
        )
        with _patched_messageable():
            await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 123)

        log_text = self._log_text()
        self.assertEqual(allowed_calls, ["ParentChannel"])
        self.assertIn("startup_channel_probe label=allowed channel=123 status=ok", log_text)
        self.assertIn("source=test_cache", log_text)
        self.assertIn("type=ParentChannel", log_text)
        self.assertIn("parent=555", log_text)
        self.assertIn("messageable=True", log_text)
        self.assertIn("allowed_message=True", log_text)

    async def test_probe_channel_access_logs_fetched_non_messageable_without_allowed_check(self) -> None:
        fetched_channel = SimpleNamespace(parent_id=777)
        allowed_calls: list[str] = []

        async def fetch_channel(channel_id: int) -> SimpleNamespace:
            _ = channel_id
            return fetched_channel

        def is_allowed_message_channel(message_channel) -> bool:
            allowed_calls.append(type(message_channel).__name__)
            return True

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (None, "-"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=is_allowed_message_channel,
        )
        with _patched_messageable():
            await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 456)

        log_text = self._log_text()
        self.assertEqual(allowed_calls, [])
        self.assertIn("startup_channel_probe label=allowed channel=456 status=ok", log_text)
        self.assertIn("source=fetch", log_text)
        self.assertIn("type=SimpleNamespace", log_text)
        self.assertIn("parent=777", log_text)
        self.assertIn("messageable=False", log_text)
        self.assertIn("allowed_message=False", log_text)

    async def test_probe_channel_access_fetch_runtime_failure_logs_and_returns(self) -> None:
        async def fetch_channel(channel_id: int) -> None:
            _ = channel_id
            raise StartupFetchUnavailableError("startup fetch unavailable")

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (None, "-"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=lambda channel: True,
        )

        await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 456)

        log_text = self._log_text()
        self.assertIn("startup_channel_probe label=allowed channel=456 status=failed", log_text)
        self.assertIn("source=fetch", log_text)
        self.assertIn("error_type=StartupFetchUnavailableError", log_text)

    async def test_probe_channel_access_fetch_type_error_is_not_probe_failure(self) -> None:
        async def fetch_channel(channel_id: int) -> None:
            _ = channel_id
            raise BadStartupFetchDependencyError("bad startup fetch dependency")

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (None, "-"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=lambda channel: True,
        )

        with self.assertRaisesRegex(BadStartupFetchDependencyError, "bad startup fetch dependency"):
            await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 456)

        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
        self.assertNotIn("startup_channel_probe label=allowed channel=456 status=failed", log_text)

    async def test_probe_channel_access_allowed_runtime_failure_logs_false(self) -> None:
        channel = ParentChannel()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached startup probe channel should not fetch", channel_id)

        def is_allowed_message_channel(message_channel) -> bool:
            _ = message_channel
            raise AllowedCheckUnavailableError("allowed check unavailable")

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=is_allowed_message_channel,
        )
        with _patched_messageable():
            await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 123)

        log_text = self._log_text()
        self.assertIn("startup_channel_probe label=allowed channel=123 status=ok", log_text)
        self.assertIn("allowed_message=False", log_text)

    async def test_probe_channel_access_allowed_type_error_is_not_allowed_false(self) -> None:
        channel = ParentChannel()

        async def fetch_channel(channel_id: int) -> None:
            await _fail_fetch("cached startup probe channel should not fetch", channel_id)

        def is_allowed_message_channel(message_channel) -> bool:
            _ = message_channel
            raise BadAllowedCheckDependencyError("bad allowed check dependency")

        client = SimpleNamespace(
            get_cached_channel_or_thread=lambda channel_id: (channel, "test_cache"),
            fetch_channel=fetch_channel,
            is_allowed_message_channel=is_allowed_message_channel,
        )
        with _patched_messageable():
            with self.assertRaisesRegex(BadAllowedCheckDependencyError, "bad allowed check dependency"):
                await bot.CodexDiscordBot.probe_channel_access(_bot_client(client), "allowed", 123)

        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
        self.assertNotIn("startup_channel_probe label=allowed channel=123 status=ok", log_text)

    async def test_log_startup_diagnostics_runtime_failure_logs_failed(self) -> None:
        async def probe_channel_access(label: str, channel_id: int) -> None:
            _ = (label, channel_id)
            raise StartupProbeUnavailableError("startup probe unavailable")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            probe_channel_access=probe_channel_access,
        )

        with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
            await bot.CodexDiscordBot.log_startup_diagnostics(_bot_client(client))

        log_text = self._log_text()
        self.assertIn("startup_diagnostics_start targets=1", log_text)
        self.assertIn("startup_diagnostics_failed", log_text)
        self.assertIn("StartupProbeUnavailableError: startup probe unavailable", log_text)

    async def test_log_startup_diagnostics_type_error_is_not_diagnostics_failed(self) -> None:
        async def probe_channel_access(label: str, channel_id: int) -> None:
            _ = (label, channel_id)
            raise BadStartupDiagnosticsDependencyError("bad startup diagnostics dependency")

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            probe_channel_access=probe_channel_access,
        )

        with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
            with self.assertRaisesRegex(
                BadStartupDiagnosticsDependencyError,
                "bad startup diagnostics dependency",
            ):
                await bot.CodexDiscordBot.log_startup_diagnostics(_bot_client(client))

        log_text = self._log_text()
        self.assertIn("startup_diagnostics_start targets=1", log_text)
        self.assertNotIn("startup_diagnostics_failed", log_text)

    async def test_log_startup_diagnostics_probe_timeout_logs_and_continues(self) -> None:
        original_timeout = bot.get_startup_channel_probe_timeout

        async def probe_channel_access(label: str, channel_id: int) -> None:
            _ = (label, channel_id)
            await asyncio.sleep(60.0)

        client = SimpleNamespace(
            allowed_channel_ids={123},
            startup_channel_id=None,
            probe_channel_access=probe_channel_access,
        )
        try:
            bot.get_startup_channel_probe_timeout = lambda: 0.01
            with mock.patch.object(bot, "get_startup_probe_targets", return_value=[("allowed", 123)]):
                await bot.CodexDiscordBot.log_startup_diagnostics(_bot_client(client))
        finally:
            bot.get_startup_channel_probe_timeout = original_timeout

        log_text = self._log_text()
        self.assertIn("startup_channel_probe label=allowed channel=123 status=timeout", log_text)
        self.assertIn("startup_diagnostics_done", log_text)


if __name__ == "__main__":
    _ = unittest.main()

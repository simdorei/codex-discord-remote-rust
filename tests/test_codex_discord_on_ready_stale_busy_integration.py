from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Never, Protocol, cast, override
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_discord_store as discord_store


class StaleBusyCleanupUnavailableError(RuntimeError):
    pass


class ReadyClient:
    def __init__(self, calls: list[str], cleanup_stale_busy_choice_components: StaleBusyCleanup) -> None:
        self.user: str = "bot#0001"
        self.guilds: list[str] = []
        self.startup_channel_id: None = None
        self.cleanup_stale_busy_choice_components: StaleBusyCleanup = cleanup_stale_busy_choice_components
        self._calls: list[str] = calls

    async def log_startup_diagnostics(self) -> None:
        self._calls.append("startup")


class StaleBusyCleanup(Protocol):
    def __call__(self) -> Awaitable[None]: ...


class OnReady(Protocol):
    def __call__(self, client: ReadyClient) -> Awaitable[None]: ...


def _on_ready() -> OnReady:
    return cast(OnReady, bot.CodexDiscordBot.on_ready)


def _noop_store_cleanup(*_args: Never, **_kwargs: Never) -> int:
    return 0


def _raise_type_error(message: str) -> Never:
    return (_ for _ in ()).throw(TypeError(message))


class DiscordOnReadyStaleBusyIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_on_ready_stale_busy_wrapper_runtime_failure_logs_and_continues(self) -> None:
        calls: list[str] = []

        async def fail_stale_busy_cleanup() -> None:
            raise StaleBusyCleanupUnavailableError("stale busy wrapper unavailable")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
        ):
            await _on_ready()(ReadyClient(calls, fail_stale_busy_cleanup))

        self.assertEqual(calls, ["startup"])
        self.assertIn("stale_busy_choice_component_cleanup_failed", self._log_text())

    async def test_on_ready_stale_busy_wrapper_type_error_is_not_cleanup_failed(self) -> None:
        calls: list[str] = []

        async def fail_stale_busy_cleanup() -> None:
            _raise_type_error("bad stale busy wrapper dependency")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
            self.assertRaisesRegex(TypeError, "bad stale busy wrapper dependency"),
        ):
            await _on_ready()(ReadyClient(calls, fail_stale_busy_cleanup))

        self.assertEqual(calls, [])
        self.assertNotIn("stale_busy_choice_component_cleanup_failed", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()

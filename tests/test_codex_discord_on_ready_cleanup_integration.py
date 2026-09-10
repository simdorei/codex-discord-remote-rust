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


class CleanupUnavailableError(RuntimeError):
    pass


class ReadyClient:
    def __init__(self, calls: list[str]) -> None:
        self.user: str = "bot#0001"
        self.guilds: list[str] = []
        self.startup_channel_id: None = None
        self._calls: list[str] = calls

    async def log_startup_diagnostics(self) -> None:
        self._calls.append("startup")


class OnReady(Protocol):
    def __call__(self, client: ReadyClient) -> Awaitable[None]: ...


def _on_ready() -> OnReady:
    return cast(OnReady, bot.CodexDiscordBot.on_ready)


class DiscordOnReadyCleanupIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _old_mirror_db_path: Path = Path()
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_mirror_db_path = bot.MIRROR_DB_PATH
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        temp_root = Path(temp_dir.name)
        bot.MIRROR_DB_PATH = temp_root / "mirror.sqlite"
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(temp_root / "discord-smoke.log")
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_mirror_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_on_ready_cleans_up_expired_busy_choices(self) -> None:
        calls: list[str] = []

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 3),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 2),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
        ):
            await _on_ready()(ReadyClient(calls))

        self.assertEqual(calls, ["startup"])
        log_text = self._log_text()
        self.assertIn("busy_choice_cleanup_deleted count=3", log_text)
        self.assertIn("persistent_component_claim_cleanup_deleted count=2", log_text)

    async def test_on_ready_cleanup_runtime_failures_log_and_continue(self) -> None:
        calls: list[str] = []

        def fail_busy() -> int:
            raise CleanupUnavailableError("busy cleanup unavailable")

        def fail_claims() -> int:
            raise CleanupUnavailableError("component claim cleanup unavailable")

        def fail_processed(*_args: Never, **_kwargs: Never) -> int:
            raise CleanupUnavailableError("processed message cleanup unavailable")

        def fail_session(*_args: Never, **_kwargs: Never) -> int:
            raise CleanupUnavailableError("session mirror event cleanup unavailable")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", fail_busy),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", fail_claims),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", fail_processed),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", fail_session),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
        ):
            await _on_ready()(ReadyClient(calls))

        self.assertEqual(calls, ["startup"])
        log_text = self._log_text()
        self.assertIn("busy_choice_cleanup_failed", log_text)
        self.assertIn("persistent_component_claim_cleanup_failed", log_text)
        self.assertIn("processed_message_cleanup_failed", log_text)
        self.assertIn("session_mirror_event_cleanup_failed", log_text)


if __name__ == "__main__":
    _ = unittest.main()

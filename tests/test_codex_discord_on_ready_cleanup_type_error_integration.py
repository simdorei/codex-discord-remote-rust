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


class ReadyClient:
    def __init__(self) -> None:
        self.user: str = "bot#0001"
        self.guilds: list[str] = []
        self.startup_channel_id: None = None

    async def log_startup_diagnostics(self) -> None:
        return None


class OnReady(Protocol):
    def __call__(self, client: ReadyClient) -> Awaitable[None]: ...


def _on_ready() -> OnReady:
    return cast(OnReady, bot.CodexDiscordBot.on_ready)


def _noop_store_cleanup(*_args: Never, **_kwargs: Never) -> int:
    return 0


def _raise_type_error(message: str) -> Never:
    return (_ for _ in ()).throw(TypeError(message))


class DiscordOnReadyCleanupTypeErrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_on_ready_busy_cleanup_type_error_is_not_cleanup_failure(self) -> None:
        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: _raise_type_error("bad busy cleanup dependency")),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            self.assertRaisesRegex(TypeError, "bad busy cleanup dependency"),
        ):
            await _on_ready()(ReadyClient())

        self.assertNotIn("busy_choice_cleanup_failed", self._log_text())

    async def test_on_ready_claim_cleanup_type_error_is_not_cleanup_failure(self) -> None:
        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: _raise_type_error("bad claim cleanup dependency")),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            self.assertRaisesRegex(TypeError, "bad claim cleanup dependency"),
        ):
            await _on_ready()(ReadyClient())

        self.assertNotIn("persistent_component_claim_cleanup_failed", self._log_text())

    async def test_on_ready_processed_cleanup_type_error_is_not_cleanup_failure(self) -> None:
        def fail_processed(*_args: Never, **_kwargs: Never) -> int:
            return _raise_type_error("bad processed cleanup dependency")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", fail_processed),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            self.assertRaisesRegex(TypeError, "bad processed cleanup dependency"),
        ):
            await _on_ready()(ReadyClient())

        self.assertNotIn("processed_message_cleanup_failed", self._log_text())

    async def test_on_ready_session_cleanup_type_error_is_not_cleanup_failure(self) -> None:
        def fail_session(*_args: Never, **_kwargs: Never) -> int:
            return _raise_type_error("bad session cleanup dependency")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", fail_session),
            self.assertRaisesRegex(TypeError, "bad session cleanup dependency"),
        ):
            await _on_ready()(ReadyClient())

        self.assertNotIn("session_mirror_event_cleanup_failed", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()

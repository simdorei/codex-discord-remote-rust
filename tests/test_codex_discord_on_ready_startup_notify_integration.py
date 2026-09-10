from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import ModuleType
from typing import Never, Protocol, cast, override
import importlib
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_discord_runtime_config as runtime_config
import codex_discord_store as discord_store


class StartupChannelUnavailableError(RuntimeError):
    pass


class StartupSendUnavailableError(RuntimeError):
    pass


class FakeMessageable:
    pass


class FakeChannel(FakeMessageable):
    pass


class FetchChannel(Protocol):
    def __call__(self, channel_id: int) -> Awaitable[FakeChannel]: ...


class StartupNotifyClient:
    def __init__(
        self,
        startup_channel_id: int,
        *,
        channel: FakeChannel | None = None,
        fetch_channel: FetchChannel | None = None,
    ) -> None:
        self.user: str = "bot#0001"
        self.guilds: list[str] = []
        self.startup_channel_id: int = startup_channel_id
        self._channel: FakeChannel | None = channel
        self._fetch_channel: FetchChannel | None = fetch_channel

    def get_channel(self, channel_id: int) -> FakeChannel | None:
        _ = channel_id
        return self._channel

    async def fetch_channel(self, channel_id: int) -> FakeChannel:
        if self._fetch_channel is None:
            return FakeChannel()
        return await self._fetch_channel(channel_id)

    async def log_startup_diagnostics(self) -> None:
        return None


class DiscordAbcModule(Protocol):
    Messageable: type[FakeMessageable]


class OnReady(Protocol):
    def __call__(self, client: StartupNotifyClient) -> Awaitable[None]: ...


def _on_ready() -> OnReady:
    return cast(OnReady, bot.CodexDiscordBot.on_ready)


def _noop_store_cleanup(*_args: Never, **_kwargs: Never) -> int:
    return 0


def _raise_type_error(message: str) -> Never:
    return (_ for _ in ()).throw(TypeError(message))


def _discord_abc() -> DiscordAbcModule:
    discord_module: ModuleType = importlib.import_module("discord")
    return cast(DiscordAbcModule, getattr(discord_module, "abc"))


class DiscordOnReadyStartupNotifyIntegrationTests(unittest.IsolatedAsyncioTestCase):
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

    def _clear_log(self) -> None:
        log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
        if log_path.exists():
            log_path.unlink()

    async def test_on_ready_startup_notify_runtime_failures_log_boundaries(self) -> None:
        async def fail_fetch(channel_id: int) -> FakeChannel:
            _ = channel_id
            raise StartupChannelUnavailableError("startup channel unavailable")

        async def fail_send(*_args: Never, **_kwargs: Never) -> int:
            raise StartupSendUnavailableError("startup send unavailable")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
            mock.patch.object(runtime_config, "discord_startup_notify_enabled", lambda: True),
            mock.patch.object(_discord_abc(), "Messageable", FakeMessageable),
        ):
            await _on_ready()(StartupNotifyClient(123, fetch_channel=fail_fetch))
            fetch_log_text = self._log_text()

            self._clear_log()
            with mock.patch.object(bot, "send_chunks", fail_send):
                await _on_ready()(StartupNotifyClient(456, channel=FakeChannel()))
            send_log_text = self._log_text()

        self.assertIn("startup_channel_fetch_failed", fetch_log_text)
        self.assertIn("startup_notify_failed", send_log_text)

    async def test_on_ready_startup_notify_fetch_type_error_is_not_notify_failure(self) -> None:
        async def fail_fetch(channel_id: int) -> FakeChannel:
            _ = channel_id
            _raise_type_error("bad startup fetch dependency")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
            mock.patch.object(runtime_config, "discord_startup_notify_enabled", lambda: True),
            mock.patch.object(_discord_abc(), "Messageable", FakeMessageable),
            self.assertRaisesRegex(TypeError, "bad startup fetch dependency"),
        ):
            await _on_ready()(StartupNotifyClient(123, fetch_channel=fail_fetch))

        self.assertNotIn("startup_channel_fetch_failed", self._log_text())

    async def test_on_ready_startup_notify_send_type_error_is_not_notify_failure(self) -> None:
        async def fail_send(*_args: Never, **_kwargs: Never) -> int:
            _raise_type_error("bad startup send dependency")

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
            mock.patch.object(runtime_config, "discord_startup_notify_enabled", lambda: True),
            mock.patch.object(_discord_abc(), "Messageable", FakeMessageable),
            mock.patch.object(bot, "send_chunks", fail_send),
            self.assertRaisesRegex(TypeError, "bad startup send dependency"),
        ):
            await _on_ready()(StartupNotifyClient(456, channel=FakeChannel()))

        self.assertNotIn("startup_notify_failed", self._log_text())


if __name__ == "__main__":
    _ = unittest.main()

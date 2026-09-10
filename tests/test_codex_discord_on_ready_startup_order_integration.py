from __future__ import annotations

from asyncio import CancelledError, Event, create_task, wait_for  # noqa: ANYIO_OK
from collections.abc import Coroutine
from types import ModuleType
from typing import Never, Protocol, cast
import importlib
import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_discord_runtime_config as runtime_config
import codex_discord_store as discord_store


class StartupOrderClient:
    def __init__(self, calls: list[str], startup_entered: Event, never_done: Event, *, startup_channel_id: int | None = None) -> None:
        self.user: str = "bot#0001"
        self.guilds: list[str] = []
        self.startup_channel_id: int | None = startup_channel_id
        self.calls: list[str] = calls
        self.startup_entered: Event = startup_entered
        self.never_done: Event = never_done

    def get_channel(self, channel_id: int) -> "FakeChannel | None":
        _ = channel_id
        return FakeChannel()

    async def start_stop_marker_watcher(self) -> None:
        self.calls.append("stop")

    async def start_history_polling(self) -> None:
        self.calls.append("history")

    async def start_session_mirroring(self) -> None:
        self.calls.append("mirror")

    async def log_startup_diagnostics(self) -> None:
        self.calls.append("startup")
        _ = self.startup_entered.set()
        _ = await self.never_done.wait()


class FakeMessageable:
    pass


class FakeChannel(FakeMessageable):
    pass


class DiscordAbcModule(Protocol):
    Messageable: type[FakeMessageable]


class OnReady(Protocol):
    def __call__(self, client: StartupOrderClient) -> Coroutine[None, None, None]: ...


def _on_ready() -> OnReady:
    return cast(OnReady, bot.CodexDiscordBot.on_ready)


def _noop_store_cleanup(*_args: Never, **_kwargs: Never) -> int:
    return 0


def _discord_abc() -> DiscordAbcModule:
    discord_module: ModuleType = importlib.import_module("discord")
    return cast(DiscordAbcModule, getattr(discord_module, "abc"))


class DiscordOnReadyStartupOrderIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def _run_until_startup(self, client: StartupOrderClient, expected_calls: list[str]) -> None:
        task = create_task(_on_ready()(client))
        try:
            _ = await wait_for(client.startup_entered.wait(), timeout=1.0)
            self.assertEqual(client.calls, expected_calls)
        finally:
            _ = task.cancel()
            try:
                await task
            except CancelledError as exc:
                self.assertIsInstance(exc, CancelledError)

    async def test_on_ready_starts_session_mirror_before_startup_diagnostics_returns(self) -> None:
        calls: list[str] = []
        startup_entered = Event()
        never_done = Event()
        client = StartupOrderClient(calls, startup_entered, never_done)

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
        ):
            await self._run_until_startup(client, ["stop", "history", "mirror", "startup"])

    async def test_on_ready_sends_startup_notify_before_session_mirror(self) -> None:
        calls: list[str] = []
        startup_entered = Event()
        never_done = Event()
        client = StartupOrderClient(calls, startup_entered, never_done, startup_channel_id=123)

        async def fake_send_chunks(*_args: Never, **_kwargs: Never) -> int:
            calls.append("startup_notify")
            return 1

        with (
            mock.patch.object(bot, "cleanup_expired_busy_choices", lambda: 0),
            mock.patch.object(bot, "cleanup_expired_persistent_component_claims", lambda: 0),
            mock.patch.object(discord_store, "cleanup_processed_discord_messages", _noop_store_cleanup),
            mock.patch.object(discord_store, "cleanup_session_mirror_events", _noop_store_cleanup),
            mock.patch.object(bot, "restore_durable_queue_runners", mock.AsyncMock(return_value=0)),
            mock.patch.object(runtime_config, "discord_startup_notify_enabled", lambda: True),
            mock.patch.object(_discord_abc(), "Messageable", FakeMessageable),
            mock.patch.object(bot, "send_chunks", fake_send_chunks),
        ):
            await self._run_until_startup(client, ["stop", "history", "startup_notify", "mirror", "startup"])


if __name__ == "__main__":
    _ = unittest.main()

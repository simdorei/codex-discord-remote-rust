from __future__ import annotations

from collections.abc import Awaitable
from types import SimpleNamespace
import unittest
from typing import Protocol, cast

import codex_discord_bot as bot

from tests.test_codex_discord_bot import FakeMessage


class HandlePrefixCommand(Protocol):
    def __call__(
        self,
        client: SimpleNamespace,
        message: FakeMessage,
        command_line: str,
    ) -> Awaitable[None]:
        ...


class RefreshDiscordBridgeSession(Protocol):
    def __call__(self, client: SimpleNamespace, *, limit: int | None = None) -> Awaitable[str]:
        ...


class DiscordPrefixBridgeIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_prefix_bridge_sync_runs_refresh(self) -> None:
        original_refresh = cast(RefreshDiscordBridgeSession, bot.refresh_discord_bridge_session)
        calls: list[tuple[SimpleNamespace, int | None]] = []
        try:
            async def fake_refresh(client: SimpleNamespace, *, limit: int | None = None) -> str:
                calls.append((client, limit))
                return "Discord bridge sync complete.\nselected_action: kept"

            bot.refresh_discord_bridge_session = fake_refresh
            message = FakeMessage()
            fake_bot = SimpleNamespace()
            handle_prefix = cast(HandlePrefixCommand, bot.handle_prefix_command)

            await handle_prefix(fake_bot, message, "bridge sync 17")

            self.assertEqual(calls, [(fake_bot, 17)])
            self.assertEqual(
                message.channel.messages,
                [
                    ("Discord bridge sync started.", None),
                    ("Discord bridge sync complete.\nselected_action: kept", None),
                ],
            )
        finally:
            bot.refresh_discord_bridge_session = original_refresh


if __name__ == "__main__":
    _ = unittest.main()

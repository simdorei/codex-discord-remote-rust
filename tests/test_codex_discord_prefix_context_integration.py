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


class BuildContextRefreshMessage(Protocol):
    def __call__(
        self,
        channel_id: int | None = None,
        *,
        limit: int = bot.CONTEXT_REFRESH_DEFAULT_LIMIT,
        max_chars: int = bot.CONTEXT_REFRESH_MAX_CHARS,
    ) -> str:
        ...


class DiscordPrefixContextIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_prefix_context_refresh_uses_bounded_refresh_builder(self) -> None:
        original_build_context_refresh = cast(
            BuildContextRefreshMessage,
            bot.build_context_refresh_message,
        )
        calls: list[tuple[int | None, int]] = []
        try:
            def fake_build_context_refresh(
                channel_id: int | None = None,
                *,
                limit: int = bot.CONTEXT_REFRESH_DEFAULT_LIMIT,
                max_chars: int = 0,
            ) -> str:
                _ = max_chars
                calls.append((channel_id, limit))
                return "bounded snapshot"

            bot.build_context_refresh_message = fake_build_context_refresh
            message = FakeMessage(channel_id=789)
            handle_prefix = cast(HandlePrefixCommand, bot.handle_prefix_command)

            await handle_prefix(SimpleNamespace(), message, "context refresh 77")
        finally:
            bot.build_context_refresh_message = original_build_context_refresh

        self.assertEqual(calls, [(789, bot.CONTEXT_REFRESH_MAX_LIMIT)])
        self.assertEqual(message.channel.messages, [("bounded snapshot", None)])


if __name__ == "__main__":
    _ = unittest.main()

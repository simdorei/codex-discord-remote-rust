from __future__ import annotations

# pyright: reportUnknownMemberType=false, reportUnknownVariableType=false
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import FakeBot, FakeMessage, FakeTarget


class DiscordPrefixBridgeActionsIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_unknown_prefix_command_response_is_bounded(self) -> None:
        message = FakeMessage()
        await bot.handle_prefix_command(FakeBot(), message, "x" * 4100)

        self.assertEqual(len(message.channel.messages), 1)
        content, view = message.channel.messages[0]
        self.assertLessEqual(len(content), 100)
        self.assertIsNone(view)
        self.assertTrue(content.startswith("Unknown command: !"))
        self.assertTrue(content.endswith("..."))

    async def test_prefix_open_abort_routes_to_shared_bridge_action(self) -> None:
        original_run_bridge_and_send = bot.run_bridge_and_send
        calls: list[tuple[list[str], str]] = []

        async def fake_run_bridge_and_send(
            target: FakeTarget,
            argv: list[str],
            title: str,
            failure_title: str | None = None,
            archive_cleanup_owner: bot.CodexDiscordBot | None = None,
        ) -> tuple[int, str]:
            _ = (failure_title, archive_cleanup_owner)
            calls.append((argv, title))
            await target.send("ok")
            return 0, "ok"

        try:
            bot.run_bridge_and_send = fake_run_bridge_and_send
            message = FakeMessage()
            await bot.handle_prefix_command(FakeBot(), message, "open_abort taxlab:1")

            self.assertEqual(calls, [(["open", "--abort", "taxlab:1"], "Open")])
            self.assertEqual(message.channel.messages, [("ok", None)])
        finally:
            bot.run_bridge_and_send = original_run_bridge_and_send


if __name__ == "__main__":
    _ = unittest.main()

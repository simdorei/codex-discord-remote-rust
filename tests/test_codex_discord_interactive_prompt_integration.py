from __future__ import annotations

# pyright: reportUnknownMemberType=false
import unittest

import codex_discord_approval_view
import codex_discord_bot as bot
from codex_discord_text import DISCORD_MAX_LEN

from tests.test_codex_discord_bot import FakeTarget


class DiscordInteractivePromptIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def test_busy_choice_message_is_single_discord_message(self) -> None:
        original_build_context_warning = bot.build_context_warning

        def fake_build_context_warning(target_thread_id: str | None) -> str:
            _ = target_thread_id
            return "warning " + ("w" * 900)

        try:
            bot.build_context_warning = fake_build_context_warning
            content = bot.build_busy_choice_message("x" * 4100, "thread-1")

            self.assertLessEqual(len(content), DISCORD_MAX_LEN)
            self.assertIn("[prompt truncated for Discord]", content)
            self.assertTrue(content.endswith("Choose the Discord action for this message."))
        finally:
            bot.build_context_warning = original_build_context_warning

    async def test_interactive_approval_prompt_with_view_is_truncated(self) -> None:
        channel = FakeTarget()
        await bot.send_interactive_prompt(
            channel,
            "thread-1",
            "taxlab:1",
            bot.INTERACTIVE_STATE_APPROVAL,
            "x" * 4100,
            [],
        )

        self.assertEqual(len(channel.messages), 1)
        content, view = channel.messages[0]
        self.assertLessEqual(len(content), DISCORD_MAX_LEN)
        self.assertTrue(content.endswith("[truncated for Discord]"))
        self.assertIsInstance(view, codex_discord_approval_view.ApprovalView)


if __name__ == "__main__":
    _ = unittest.main()

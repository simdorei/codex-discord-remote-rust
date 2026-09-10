from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import cast
from unittest import mock
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch
from tests.test_codex_discord_restart_notice_integration import FakeClient, FakeMessage, MessageHandler


class MirroredThreadMessageIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_accepts_bridge_mention_with_other_bot_mention(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1511380398914142379}
        message = FakeMessage(
            content="<@1511380398914142379> ask <@1500506752234422322>",
            channel_id=333,
        )
        message.raw_mentions = [1511380398914142379, 1500506752234422322]
        message.mentions = [
            SimpleNamespace(id=1511380398914142379, bot=True),
            SimpleNamespace(id=1500506752234422322, bot=True),
        ]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "describe_mirrored_project_channel", return_value=None),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "ask <@1500506752234422322>", "thread-1")])
        self.assertNotIn("other_bot_mention_in_mirrored_thread", log_text)


if __name__ == "__main__":
    _ = unittest.main()

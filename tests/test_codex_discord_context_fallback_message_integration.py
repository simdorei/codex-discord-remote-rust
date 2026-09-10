from __future__ import annotations

from pathlib import Path
from typing import cast
from unittest import mock
import tempfile
import unittest

import codex_discord_bot as bot
import codex_discord_message_gate as message_gate

from tests.test_codex_discord_bot import EnvPatch
from tests.test_codex_discord_restart_notice_integration import FakeClient, FakeMessage, MessageHandler


class ContextFallbackMessageIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_accepts_unmentioned_plain_ask(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        async def runner_idle(target_thread_id: str) -> bool:
            _ = target_thread_id
            return False

        def idle_busy_state(target_thread_id: str) -> tuple[str, None, str]:
            _ = target_thread_id
            return ("idle", None, "")

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="codex explain this in Korean", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "1"),
                EnvPatch(
                    "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
                    message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
                ),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
                mock.patch.object(bot, "describe_mirrored_project_channel", return_value=None),
                mock.patch.object(bot, "get_busy_state_for_thread", side_effect=idle_busy_state),
                mock.patch.object(bot, "is_thread_runner_busy", side_effect=runner_idle),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "codex explain this in Korean", None)])
        self.assertIn("plain_ask_context_fallback chat=333", log_text)

    async def test_accepts_korean_discord_ops_chatter(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        async def runner_idle(target_thread_id: str) -> bool:
            _ = target_thread_id
            return False

        def idle_busy_state(target_thread_id: str) -> tuple[str, None, str]:
            _ = target_thread_id
            return ("idle", None, "")

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="디코 봇 응답 없어", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "1"),
                EnvPatch(
                    "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
                    message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
                ),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
                mock.patch.object(bot, "describe_mirrored_project_channel", return_value=None),
                mock.patch.object(bot, "get_busy_state_for_thread", side_effect=idle_busy_state),
                mock.patch.object(bot, "is_thread_runner_busy", side_effect=runner_idle),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "디코 봇 응답 없어", None)])
        self.assertIn("plain_ask_context_fallback chat=333", log_text)


if __name__ == "__main__":
    _ = unittest.main()

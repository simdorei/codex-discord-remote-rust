from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import NoReturn, cast
from unittest import mock
import tempfile
import unittest

import codex_discord_bot as bot
import codex_discord_message_gate as message_gate
from codex_discord_message_gate import MessageWithMentions

from tests.test_codex_discord_bot import EnvPatch
from tests.test_codex_discord_restart_notice_integration import FakeClient, FakeMessage, MessageHandler


class RequiredMentionIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_ignores_unmentioned_plain_ask(self) -> None:
        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("unmentioned plain asks must not reach Codex")

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="plain channel chatter", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "0"),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=required_mention_missing chat=333", log_text)

    async def test_mention_only_message_prompts_for_content(self) -> None:
        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("mention-only messages must not reach Codex")

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="<@!1500506752234422322>", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [("Add a prompt after the mention.", None)])
        self.assertIn("ignored_message reason=mention_only_content chat=333", log_text)

    async def test_strips_prompt_for_plain_ask(self) -> None:
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
        message = FakeMessage(content="<@1500506752234422322> please run", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "0"),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "describe_mirrored_project_channel", return_value=None),
                mock.patch.object(bot, "get_busy_state_for_thread", side_effect=idle_busy_state),
                mock.patch.object(bot, "is_thread_runner_busy", side_effect=runner_idle),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "please run", "thread-1")])
        self.assertIn("text_len=10", log_text)

    async def test_required_mention_does_not_gate_prefix_commands(self) -> None:
        calls: list[str] = []

        async def runner_idle(target_thread_id: str) -> bool:
            _ = target_thread_id
            return False

        def idle_busy_state(target_thread_id: str) -> tuple[str, None, str]:
            _ = target_thread_id
            return ("idle", None, "")

        async def capture_handle_prefix_command(
            client: FakeClient,
            message: FakeMessage,
            command: str,
        ) -> None:
            _ = (client, message)
            calls.append(command)

        def fail_prepare_plain_ask_content(
            message: MessageWithMentions,
            content: str,
            required_user_ids: set[int],
            target_thread_id: str | None,
            *,
            has_attachments: bool,
        ) -> NoReturn:
            _ = (message, content, required_user_ids, target_thread_id, has_attachments)
            raise AssertionError("prefix commands must not enter the plain ask gate")

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="!qa buttons", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with (
            mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value=None),
            mock.patch.object(bot, "get_busy_state_for_thread", side_effect=idle_busy_state),
            mock.patch.object(bot, "is_thread_runner_busy", side_effect=runner_idle),
            mock.patch.object(bot, "handle_prefix_command", side_effect=capture_handle_prefix_command),
            mock.patch.object(
                message_gate,
                "prepare_plain_ask_content",
                side_effect=fail_prepare_plain_ask_content,
            ),
        ):
            await on_message(client, message)

        self.assertEqual(calls, ["qa buttons"])

    async def test_mirrored_thread_bypasses_required_mention(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1500506752234422322}
        message = FakeMessage(content="짧은 확인", channel_id=333)
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "0"),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "describe_mirrored_project_channel", return_value=None),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "짧은 확인", "thread-1")])
        self.assertIn("target_source=mirror target=thread-1", log_text)
        self.assertNotIn("required_mention_missing", log_text)

    async def test_mirrored_thread_ignores_other_bot_mention_without_bridge_mention(self) -> None:
        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("other bot mentions in mirrored threads must not reach Codex")

        client = FakeClient()
        client.plain_ask_mention_user_ids = {1511380398914142379}
        message = FakeMessage(content="<@1500506752234422322> ping", channel_id=333)
        message.raw_mentions = [1500506752234422322]
        message.mentions = [SimpleNamespace(id=1500506752234422322, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(client, message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=other_bot_mention_in_mirrored_thread", log_text)


if __name__ == "__main__":
    _ = unittest.main()

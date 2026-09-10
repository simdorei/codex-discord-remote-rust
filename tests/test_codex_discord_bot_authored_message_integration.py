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


class BotAuthoredMessageIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_accepts_allowed_bot_restart_check_handoff_without_bridge_mention(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []
        explicit_thread_id = "019ef4b8-c325-7a70-8781-bdcc5b21a653"

        def mirrored_codex_thread(channel_id: int | None) -> str:
            _ = channel_id
            return "thread-1"

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        message = FakeMessage(
            content=(
                "<@***> <@242286902982606848>\n"
                "ACTION: RESTART-CHECK / HANDOFF\n\n"
                f"codex/session: `{explicit_thread_id}`\n"
                "Post-restart checks:\n"
                "1. Run status.ps1\n"
                "Continuation rule: Continue refactoring until the user says to stop."
            ),
            channel_id=333,
        )
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [242286902982606848]
        message.mentions = [SimpleNamespace(id=242286902982606848, bot=False)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=mirrored_codex_thread),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(FakeClient(allowed_user_id=1500506752234422322), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, message.content, explicit_thread_id)])
        self.assertIn("bot_bridge_unmentioned_restart_check_handoff_accepted", log_text)
        self.assertNotIn("bot_author_without_bridge_mention", log_text)

    async def test_rejects_unallowed_bot_when_it_mentions_bridge_user(self) -> None:
        def mirrored_codex_thread(channel_id: int | None) -> str:
            _ = channel_id
            return "thread-1"

        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("unallowed bot bridge mentions must not reach Codex")

        message = FakeMessage(content="<@1511380398914142379> relay this", channel_id=333)
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379]
        message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=mirrored_codex_thread),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(FakeClient(allowed_user_id=0), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=user_not_allowed user=1500506752234422322", log_text)

    async def test_ignores_other_bot_without_bridge_mention(self) -> None:
        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("unmentioned bot-authored messages must not reach Codex")

        message = FakeMessage(content="<@1500506752234422322> ping", channel_id=333)
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1500506752234422322]
        message.mentions = [SimpleNamespace(id=1500506752234422322, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(FakeClient(), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=bot_author_without_bridge_mention", log_text)

    async def test_accepts_allowed_bot_when_it_mentions_bridge_user(self) -> None:
        calls: list[tuple[FakeMessage, str, str | None]] = []

        def mirrored_codex_thread(channel_id: int | None) -> str:
            _ = channel_id
            return "thread-1"

        def no_project_channel_description(channel_id: int | None) -> None:
            _ = channel_id

        async def capture_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        message = FakeMessage(content="<@1511380398914142379> relay this", channel_id=333)
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379]
        message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=mirrored_codex_thread),
                mock.patch.object(
                    bot,
                    "describe_mirrored_project_channel",
                    side_effect=no_project_channel_description,
                ),
                mock.patch.object(bot, "handle_plain_ask", side_effect=capture_handle_plain_ask),
            ):
                await on_message(FakeClient(allowed_user_id=1500506752234422322), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(message, "relay this", "thread-1")])
        self.assertNotIn("bot_author_without_bridge_mention", log_text)
        self.assertNotIn("user_not_allowed", log_text)

    async def test_ignores_bot_authored_bridge_operational_packet(self) -> None:
        def mirrored_codex_thread(channel_id: int | None) -> str:
            _ = channel_id
            return "thread-1"

        async def fail_handle_plain_ask(
            message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)
            raise AssertionError("bot bridge operational packets must not reach Codex")

        message = FakeMessage(
            content="<@1511380398914142379>\nPROGRESS: RESTART-WATCH heartbeat.",
            channel_id=333,
        )
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379]
        message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "get_mirrored_codex_thread_id", side_effect=mirrored_codex_thread),
                mock.patch.object(bot, "handle_plain_ask", side_effect=fail_handle_plain_ask),
            ):
                await on_message(FakeClient(allowed_user_id=1500506752234422322), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=bot_bridge_operational_packet", log_text)

    async def test_bot_bridge_mention_strips_prefix_command_mention(self) -> None:
        async def fake_build_runners_message() -> str:
            return "queues ok"

        message = FakeMessage(content="!runners <@1511380398914142379>", channel_id=333)
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379]
        message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "build_runners_message", side_effect=fake_build_runners_message),
            ):
                await on_message(FakeClient(allowed_user_id=1500506752234422322), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [("queues ok", None)])
        self.assertIn("bot_bridge_prefix_mention_stripped chat=333", log_text)
        self.assertNotIn("user_not_allowed", log_text)

    async def test_rejects_unallowed_bot_bridge_prefix_command(self) -> None:
        async def fail_build_runners_message() -> str:
            raise AssertionError("unallowed bot prefix commands must not dispatch")

        message = FakeMessage(content="!runners <@1511380398914142379>", channel_id=333)
        message.author = SimpleNamespace(id=1500506752234422322, bot=True)
        message.raw_mentions = [1511380398914142379]
        message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]
        on_message = cast(MessageHandler, bot.CodexDiscordBot.on_message)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                mock.patch.object(bot, "build_runners_message", side_effect=fail_build_runners_message),
            ):
                await on_message(FakeClient(allowed_user_id=0), message)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(message.channel.messages, [])
        self.assertIn("ignored_message reason=user_not_allowed user=1500506752234422322", log_text)
        self.assertNotIn("bot_bridge_prefix_mention_stripped", log_text)


if __name__ == "__main__":
    _ = unittest.main()

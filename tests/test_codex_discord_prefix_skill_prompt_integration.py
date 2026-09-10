from __future__ import annotations

from pathlib import Path
from types import SimpleNamespace
from typing import cast
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeMessage


def _prefix_bot() -> bot.CodexDiscordBot:
    return cast(bot.CodexDiscordBot, cast(object, SimpleNamespace()))


def _discord_message(message: FakeMessage) -> bot.discord.Message:
    return cast(bot.discord.Message, cast(object, message))


class PrefixSkillPromptBotIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_prefix_pro_routes_to_plain_ask_when_project_is_mirrored(self) -> None:
        # Given
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_handle_plain_ask = bot.handle_plain_ask
        calls: list[tuple[bot.discord.Message, str, str | None]] = []

        async def fake_handle_plain_ask(
            message: bot.discord.Message,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        try:
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1"
            bot.handle_plain_ask = fake_handle_plain_ask
            message = FakeMessage(content="!pro read README", channel_id=222)

            # When
            await bot.handle_prefix_command(
                _prefix_bot(),
                _discord_message(message),
                "pro read README",
            )

            # Then
            self.assertEqual(calls, [(message, "!pro read README", "thread-1")])
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.handle_plain_ask = original_handle_plain_ask

    async def test_prefix_interview_wraps_request_for_deep_interview(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_handle_plain_ask = bot.handle_plain_ask
        calls: list[tuple[bot.discord.Message, str, str | None]] = []

        async def fake_handle_plain_ask(
            message: bot.discord.Message,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        try:
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1"
            bot.handle_plain_ask = fake_handle_plain_ask
            message = FakeMessage(content="!interview build a dashboard", channel_id=222)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await bot.handle_prefix_command(
                        _prefix_bot(),
                        _discord_message(message),
                        "interview build a dashboard",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(calls), 1)
            source_message, prompt, target_thread_id = calls[0]
            self.assertIs(source_message, message)
            self.assertEqual(target_thread_id, "thread-1")
            self.assertIn("Run a Gajae-style deep interview before implementation.", prompt)
            self.assertIn("Round 0: enumerate 1-6 top-level components", prompt)
            self.assertIn("Deep Interview threshold: 0.05 (source: default)", prompt)
            self.assertIn("ontology stability", prompt)
            self.assertIn("User request:\nbuild a dashboard", prompt)
            self.assertIn("prefix_interview channel=222", log_text)
            self.assertIn("target=thread-1", log_text)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.handle_plain_ask = original_handle_plain_ask

    async def test_removed_triage_and_orchestrate_prefixes_fall_through_to_unknown(self) -> None:
        original_handle_plain_ask = bot.handle_plain_ask
        calls: list[str] = []

        async def fake_handle_plain_ask(
            message: bot.discord.Message,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = message, target_thread_id
            calls.append(prompt)

        try:
            bot.handle_plain_ask = fake_handle_plain_ask
            cases = [
                ("triage current repo", "Unknown command: !triage"),
                ("orchestrate inspect queue", "Unknown command: !orchestrate"),
            ]

            for content, expected_message in cases:
                with self.subTest(content=content):
                    message = FakeMessage(content=f"!{content}", channel_id=222)

                    await bot.handle_prefix_command(
                        _prefix_bot(),
                        _discord_message(message),
                        content,
                    )

                    self.assertEqual(message.channel.messages[-1][0], expected_message)

            self.assertEqual(calls, [])
        finally:
            bot.handle_plain_ask = original_handle_plain_ask


if __name__ == "__main__":
    unittest.main()

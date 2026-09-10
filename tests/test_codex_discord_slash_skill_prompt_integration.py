# pyright: reportArgumentType=false, reportUnknownArgumentType=false, reportUnknownMemberType=false
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import os
import tempfile
import unittest
from unittest import mock

import codex_discord_bot as bot


class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, bool]] = []

    async def send(self, content: str, **kwargs: bool) -> None:
        self.messages.append(content)
        self.kwargs.append(kwargs)


class FakeTarget:
    def __init__(self, channel_id: int = 222) -> None:
        self.id: int = channel_id
        self.messages: list[str] = []

    async def send(self, content: str, view: None = None) -> None:
        _ = view
        self.messages.append(content)


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int = 242286902982606848


class FakeInteraction:
    def __init__(self, command_name: str, channel_id: int = 222) -> None:
        self.command: FakeCommand = FakeCommand(command_name)
        self.channel_id: int = channel_id
        self.followup: FakeFollowup = FakeFollowup()
        self.user: FakeUser = FakeUser()
        self.channel: FakeTarget = FakeTarget(channel_id)


class DiscordSlashSkillPromptIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_interview_wraps_request_for_deep_interview(self) -> None:
        calls: list[tuple[bot.SlashAskSourceMessage, str, str | None]] = []
        interaction = FakeInteraction(command_name="interview", channel_id=222)

        async def fake_handle_plain_ask(
            message: bot.SlashAskSourceMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            calls.append((message, prompt, target_thread_id))

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot, "get_mirrored_codex_thread_id", return_value="thread-1"),
                mock.patch.object(bot, "handle_plain_ask", fake_handle_plain_ask),
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
            ):
                await bot.handle_slash_interview(interaction, "build a dashboard")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(interaction.followup.messages, ["Interview handling posted in this channel."])
        self.assertEqual(interaction.followup.kwargs, [{"ephemeral": True}])
        self.assertEqual(len(calls), 1)
        source_message, prompt, target_thread_id = calls[0]
        self.assertEqual(target_thread_id, "thread-1")
        self.assertIs(source_message.channel, interaction.channel)
        self.assertIn("Run a Gajae-style deep interview before implementation.", prompt)
        self.assertIn("Round 0: enumerate 1-6 top-level components", prompt)
        self.assertIn("Deep Interview threshold: 0.05 (source: default)", prompt)
        self.assertIn("challenge modes", prompt)
        self.assertIn("User request:\nbuild a dashboard", prompt)
        self.assertIn("slash_interview_dispatch command=interview channel=222", log_text)
        self.assertIn("target_source=mirror target=thread-1", log_text)

    def test_removed_slash_skill_wrappers_are_absent(self) -> None:
        self.assertFalse(hasattr(bot, "handle_slash_github_triage"))
        self.assertFalse(hasattr(bot, "handle_slash_maintainer_orchestrator"))


if __name__ == "__main__":
    _ = unittest.main()

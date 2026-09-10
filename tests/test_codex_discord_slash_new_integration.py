# pyright: reportArgumentType=false, reportUnknownMemberType=false
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


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int = 242286902982606848


class FakeInteraction:
    def __init__(self, command_name: str = "new", channel_id: int = 222) -> None:
        self.command: FakeCommand = FakeCommand(command_name)
        self.channel_id: int = channel_id
        self.followup: FakeFollowup = FakeFollowup()
        self.user: FakeUser = FakeUser()


class FakeBot:
    pass


class DiscordSlashNewIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_dispatch_logs_and_sends_response(self) -> None:
        calls: list[tuple[FakeBot, int | None, str]] = []
        fake_bot = FakeBot()
        interaction = FakeInteraction(command_name="new", channel_id=222)

        async def fake_run_discord_new_thread(
            sent_bot: FakeBot,
            channel_id: int | None,
            prompt: str,
        ) -> tuple[int, str]:
            calls.append((sent_bot, channel_id, prompt))
            return 0, "New\n\ntarget_thread: thread-new"

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot, "run_discord_new_thread", fake_run_discord_new_thread),
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
            ):
                await bot.handle_slash_new(fake_bot, interaction, "start here")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(calls, [(fake_bot, 222, "start here")])
        self.assertEqual(interaction.followup.messages, ["New\n\ntarget_thread: thread-new"])
        self.assertEqual(interaction.followup.kwargs, [{}])
        self.assertIn("slash_new_dispatch channel=222", log_text)
        self.assertIn("user=242286902982606848", log_text)
        self.assertIn("prompt_len=10", log_text)
        self.assertIn("slash_new_done channel=222 exit=0", log_text)
        self.assertIn("slash_response_start command=new title='New' exit=0", log_text)
        self.assertIn("slash_response_sent command=new title='New' exit=0", log_text)


if __name__ == "__main__":
    _ = unittest.main()

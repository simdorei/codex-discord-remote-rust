from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast, runtime_checkable
import tempfile
import unittest

import discord

import codex_discord_bot as bot
from codex_discord_text import DISCORD_MAX_LEN

from tests.test_codex_discord_bot import EnvPatch, FakeInteraction


@runtime_checkable
class ApprovalButton(Protocol):
    @property
    def label(self) -> str | None: ...

    async def callback(self, interaction: FakeInteraction, /) -> None:
        ...


class StreamPostApproval(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        watch_result: bot.SteeringPromptResult | None,
        target_thread_id: str,
    ) -> Awaitable[bool]:
        ...


def find_approval_button(view: discord.ui.View) -> ApprovalButton:
    for item in view.children:
        if isinstance(item, ApprovalButton) and item.label == "Approve":
            return item
    raise StopIteration("Approve button not found")


class DiscordApprovalButtonIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_approval_button_chunks_long_output(self) -> None:
        original_submit_approval_reply = bot.submit_approval_reply
        try:
            def fake_submit_approval_reply(target_thread_id: str, answer: str) -> tuple[int, str]:
                _ = target_thread_id, answer
                return 0, "approved\n" + ("x" * 4100)

            bot.submit_approval_reply = fake_submit_approval_reply
            interaction = FakeInteraction(command_name="approval", channel_id=222)
            view = bot.ApprovalView("thread-1")
            button = find_approval_button(view)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertGreater(len(interaction.followup.messages), 1)
            self.assertTrue(all(len(str(message)) <= DISCORD_MAX_LEN for message in interaction.followup.messages))
            self.assertIn("Approval submitted", str(interaction.followup.messages[0]))
            self.assertIn("button_response_start command=approval title='Approval' exit=0", log_text)
            self.assertIn("approval_button_sent exit=0 target=thread-1", log_text)
            self.assertIn("approval_button user=242286902982606848 answer_len=1", log_text)
            self.assertIn("approval_button_done exit=0 target=thread-1 answer_len=1", log_text)
            self.assertNotIn("answer=1", log_text)
        finally:
            bot.submit_approval_reply = original_submit_approval_reply

    async def test_approval_button_starts_post_approval_watch(self) -> None:
        original_submit_approval_reply = bot.submit_approval_reply
        original_make_watch = bot.make_post_approval_watch_result
        original_stream_watch = cast(StreamPostApproval, bot.stream_post_approval_result_for_interaction)
        calls: list[tuple[str, str | None, bot.SteeringPromptResult | None]] = []
        watch_result = bot.SteeringPromptResult(
            0,
            "[approval_submitted]",
            target_thread_id="thread-1",
            target_ref="project:1",
            session_path="session.jsonl",
            start_offset=10,
        )
        try:
            def fake_submit_approval_reply(target_thread_id: str, answer: str) -> tuple[int, str]:
                _ = target_thread_id, answer
                return 0, "approved"

            def fake_make_watch(target_thread_id: str) -> bot.SteeringPromptResult:
                calls.append(("make", target_thread_id, None))
                return watch_result

            async def fake_stream_watch(
                interaction: FakeInteraction,
                watch: bot.SteeringPromptResult,
                target_thread_id: str | None,
            ) -> bool:
                _ = interaction
                calls.append(("stream", target_thread_id, watch))
                return True

            bot.submit_approval_reply = fake_submit_approval_reply
            bot.make_post_approval_watch_result = fake_make_watch
            bot.stream_post_approval_result_for_interaction = fake_stream_watch
            interaction = FakeInteraction(command_name="approval", channel_id=222)
            view = bot.ApprovalView("thread-1")
            button = find_approval_button(view)

            await button.callback(interaction)

            self.assertEqual(interaction.followup.messages, ["Approval submitted\n\napproved"])
            self.assertEqual(
                calls,
                [
                    ("make", "thread-1", None),
                    ("stream", "thread-1", watch_result),
                ],
            )
        finally:
            bot.submit_approval_reply = original_submit_approval_reply
            bot.make_post_approval_watch_result = original_make_watch
            bot.stream_post_approval_result_for_interaction = original_stream_watch


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
import tempfile
import unittest
from typing import Protocol, cast

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeMessage, FakeTarget
from tests.test_codex_discord_plain_ask_direct_integration import handle_plain_ask


class MessageableLike(Protocol):
    async def send(self, content: str, view: bot.ApprovalView | None = None) -> None:
        ...


class SubmitInteractiveReply(Protocol):
    def __call__(
        self,
        channel: MessageableLike,
        target_thread_id: str,
        target_ref: str,
        state: str,
        answer: str,
    ) -> Awaitable[None]:
        ...


class SubmitApprovalReply(Protocol):
    def __call__(self, target_thread_id: str, answer: str) -> tuple[int, str]:
        ...


def approval_interactive_state(target_thread_id: str | None) -> tuple[str, str, str]:
    _ = target_thread_id
    return bot.INTERACTIVE_STATE_APPROVAL, "thread-1", "project:1"


class DiscordPendingApprovalPlainIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_pending_approval_plain_text_refreshes_buttons_without_submitting(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_submit_interactive_reply = cast(SubmitInteractiveReply, bot.submit_interactive_reply)
        submitted: list[str] = []
        try:
            async def fake_submit(
                channel: MessageableLike,
                target_thread_id: str,
                target_ref: str,
                state: str,
                answer: str,
            ) -> None:
                _ = channel, target_thread_id, target_ref, state
                submitted.append(answer)

            bot.get_interactive_state_for_thread = approval_interactive_state
            bot.submit_interactive_reply = fake_submit
            message = FakeMessage()

            await handle_plain_ask()(message, "new steering request", target_thread_id="thread-1")

            self.assertEqual(submitted, [])
            self.assertEqual(len(message.channel.messages), 1)
            content, view = message.channel.messages[0]
            self.assertIn("Waiting approval", content)
            self.assertIn("Pending approval", content)
            self.assertIsInstance(view, bot.ApprovalView)
            approval_view = cast(bot.ApprovalView, view)
            self.assertEqual(approval_view.target_thread_id, "thread-1")
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.submit_interactive_reply = original_submit_interactive_reply

    async def test_pending_approval_plain_numeric_reply_still_submits(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_submit_interactive_reply = cast(SubmitInteractiveReply, bot.submit_interactive_reply)
        submitted: list[str] = []
        try:
            async def fake_submit(
                channel: MessageableLike,
                target_thread_id: str,
                target_ref: str,
                state: str,
                answer: str,
            ) -> None:
                _ = channel, target_thread_id, target_ref, state
                submitted.append(answer)

            bot.get_interactive_state_for_thread = approval_interactive_state
            bot.submit_interactive_reply = fake_submit
            message = FakeMessage()

            await handle_plain_ask()(message, "approve", target_thread_id="thread-1")

            self.assertEqual(submitted, ["1"])
            self.assertEqual(message.channel.messages, [])
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.submit_interactive_reply = original_submit_interactive_reply

    async def test_plain_approval_reply_log_uses_answer_length(self) -> None:
        original_submit_approval_reply = cast(SubmitApprovalReply, bot.submit_approval_reply)
        try:
            def fake_submit_approval_reply(target_thread_id: str, answer: str) -> tuple[int, str]:
                _ = target_thread_id, answer
                return 0, "approved"

            bot.submit_approval_reply = fake_submit_approval_reply
            channel = FakeTarget()
            secret_answer = "approve this sensitive text"
            submit_reply = cast(SubmitInteractiveReply, bot.submit_interactive_reply)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await submit_reply(
                        cast(MessageableLike, channel),
                        "thread-1",
                        "taxlab:1",
                        bot.INTERACTIVE_STATE_APPROVAL,
                        secret_answer,
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(channel.messages, [("Approval submitted\n\napproved", None)])
            self.assertIn("approval_reply_done exit=0 target=thread-1", log_text)
            self.assertIn(f"answer_len={len(secret_answer)}", log_text)
            self.assertNotIn(secret_answer, log_text)
        finally:
            bot.submit_approval_reply = original_submit_approval_reply


if __name__ == "__main__":
    _ = unittest.main()

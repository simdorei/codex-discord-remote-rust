from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class DiscordAskDeliveryPendingIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_ask_stream_delivery_pending_does_not_send_failure(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        try:
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait, target_thread_id
                return (
                    1,
                    "\n".join(
                        [
                            "target_thread: thread-1",
                            "ui_activation: ipc-thread-follower-start-turn",
                            "ERROR: Prompt delivery could not be confirmed in any recent Codex thread after IPC delivery. The transport reported success, but no matching user message was recorded.",
                        ]
                    ),
                )

            bot.run_ask_stream = fake_run_ask_stream

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await run_prompt_and_send()(
                        message.channel,
                        "please run",
                        ack_sent=True,
                        source_message=message,
                        target_thread_id="thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(message.channel.messages), 1)
            content, view = message.channel.messages[0]
            self.assertIn("[delivery_pending]", content)
            self.assertIn("Wait for the mirrored reply before resending.", content)
            self.assertNotIn("Ask failed", content)
            self.assertNotIn("ERROR:", content)
            self.assertIsNone(view)
            self.assertIn("ask_stream_delivery_pending exit=1 target=thread-1", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_ask_stream_start_turn_timeout_does_not_send_failure(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        try:
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait, target_thread_id
                return (
                    1,
                    "\n".join(
                        [
                            "target_thread: thread-1",
                            "ui_activation: ipc-thread-follower-start-turn",
                            "ERROR: IPC start-turn failed: thread-follower-start-turn-timeout",
                        ]
                    ),
                )

            bot.run_ask_stream = fake_run_ask_stream

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await run_prompt_and_send()(
                        message.channel,
                        "please run",
                        ack_sent=True,
                        source_message=message,
                        target_thread_id="thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(message.channel.messages), 1)
            content, view = message.channel.messages[0]
            self.assertIn("[delivery_pending]", content)
            self.assertIn("Wait for the mirrored reply before resending.", content)
            self.assertNotIn("thread-follower-start-turn-timeout", content)
            self.assertNotIn("Ask failed", content)
            self.assertIsNone(view)
            self.assertIn("ask_stream_delivery_pending exit=1 target=thread-1", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream


if __name__ == "__main__":
    _ = unittest.main()

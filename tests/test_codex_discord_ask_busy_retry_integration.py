from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class DiscordAskBusyRetryIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_ask_target_busy_failure_retries_without_queueing(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_get_retry_delay = bot.get_ask_busy_retry_delay_seconds
        calls: list[bool] = []
        try:
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.get_interactive_state_for_thread = lambda target_thread_id: ("", None, "")
            bot.get_ask_busy_retry_delay_seconds = lambda: 0

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, wait, target_thread_id
                calls.append(force_while_busy)
                return (
                    1,
                    "\n".join(
                        [
                            "Ask failed (exit 1)",
                            "",
                            "ERROR: The selected thread is still busy. Wait, switch to another thread, or pass --force-while-busy.",
                        ]
                    ),
                )

            bot.run_ask_stream = fake_run_ask_stream
            bot.build_context_warning = lambda target_thread_id: ""

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_ASK_BUSY_RETRY_ATTEMPTS", "1"):
                        await run_prompt_and_send()(
                            message.channel,
                            "please steer",
                            ack_sent=True,
                            source_message=message,
                            target_thread_id="thread-1",
                        )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(message.channel.messages), 2)
            retry_notice, retry_view = message.channel.messages[0]
            final_status, final_view = message.channel.messages[1]
            self.assertIn("Retrying mapped-thread delivery up to 1 time(s).", retry_notice)
            self.assertIsNone(retry_view)
            self.assertIn("Codex app did not accept this Discord message yet.", final_status)
            self.assertIsNone(final_view)
            self.assertEqual(calls, [False, False])
            self.assertIn("ask_stream_retry_done attempt=1", log_text)
            self.assertNotIn("busy_choice_sent reason=ask_target_busy_failure", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.get_ask_busy_retry_delay_seconds = original_get_retry_delay


if __name__ == "__main__":
    _ = unittest.main()

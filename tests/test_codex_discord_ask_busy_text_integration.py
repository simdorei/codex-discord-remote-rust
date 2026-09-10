from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class DiscordAskBusyTextIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_other_thread_busy_transport_text_is_plain_failure(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_get_interactive_state = bot.get_interactive_state_for_thread
        try:
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.get_interactive_state_for_thread = lambda target_thread_id: ("", None, "")

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
                            "Ask failed (exit 1)",
                            "",
                            "ERROR: Another mapped thread is still working.",
                        ]
                    ),
                )

            bot.run_ask_stream = fake_run_ask_stream
            bot.build_context_warning = lambda target_thread_id: ""

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_ASK_BUSY_RETRY_ATTEMPTS", "0"):
                        await run_prompt_and_send()(
                            message.channel,
                            "please steer",
                            ack_sent=True,
                            source_message=message,
                            target_thread_id="thread-1",
                        )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(message.channel.messages), 1)
            content, view = message.channel.messages[0]
            self.assertIn("Ask failed (exit 1)", content)
            self.assertIsNone(view)
            self.assertNotIn("Choose the Discord action", content)
            self.assertNotIn("ask_stream_busy_transport_failure", log_text)
            self.assertNotIn("busy_choice_sent", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream
            bot.build_context_warning = original_build_context_warning
            bot.get_interactive_state_for_thread = original_get_interactive_state


if __name__ == "__main__":
    _ = unittest.main()

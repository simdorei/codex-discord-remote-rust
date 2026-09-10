from __future__ import annotations

from pathlib import Path
import tempfile
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class DiscordAskStreamRelayIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_ask_stream_stale_relay_suppresses_fallback_after_newer_relay(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_generations = dict(bot.get_runtime_state().active_discord_relay_generations)
        try:
            bot.get_runtime_state().active_discord_relay_generations.clear()
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, force_while_busy, wait
                _ = bot.DiscordAskRelay(
                    relay.loop,
                    relay.channel,
                    target_thread_id,
                    "taxlab:1",
                )
                relay.feed_line("[final_answer]")
                relay.feed_line("stale final")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\nstale final\n\n[ready]"

            bot.run_ask_stream = fake_run_ask_stream

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await run_prompt_and_send()(
                        message.channel,
                        "please steer",
                        ack_sent=True,
                        source_message=message,
                        target_thread_id="thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(message.channel.messages, [])
            self.assertIn("discord_relay_suppressed_after_steering target=thread-1 mode=final", log_text)
            self.assertIn("ask_stream_suppressed_after_newer_relay target=thread-1", log_text)
            self.assertNotIn("ask_stream_no_final_fallback", log_text)
        finally:
            bot.get_runtime_state().active_discord_relay_generations.clear()
            bot.get_runtime_state().active_discord_relay_generations.update(old_generations)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_run_prompt_and_send_uses_typing_indicator(self) -> None:
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
                _ = prompt, force_while_busy, wait, target_thread_id
                relay.feed_line("[final_answer]")
                relay.feed_line("done")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\ndone\n\n[ready]"

            bot.run_ask_stream = fake_run_ask_stream

            message = FakeMessage()
            await run_prompt_and_send()(
                message.channel,
                "please run",
                ack_sent=True,
                source_message=message,
                target_thread_id="thread-1",
            )

            self.assertEqual(message.channel.typing_events, ["enter", "exit"])
            self.assertEqual(message.channel.messages, [("Final\n\ndone", None)])
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream


if __name__ == "__main__":
    _ = unittest.main()

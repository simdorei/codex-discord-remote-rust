from __future__ import annotations

from pathlib import Path
import tempfile
import time
import unittest

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeMessage


def activate_steering_handoff(target_thread_id: str | None) -> None:
    bot.get_runtime_state().steering_handoffs[
        discord_runtime.normalize_runner_key(target_thread_id)
    ] = time.monotonic() + 1.0


class DiscordAskStreamHandoffIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_ask_stream_live_without_final_reports_error(self) -> None:
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
                relay.feed_line("[commentary]")
                relay.feed_line("Still compacting context.")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[commentary]\nStill compacting context.\n\n[ready]"

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

            sent = [content for content, _view in message.channel.messages]
            self.assertNotIn("Codex turn finished.", sent)
            self.assertTrue(any("Ask failed" in content for content in sent))
            self.assertTrue(any("ERROR: Codex stream completed without a final answer." in content for content in sent))
            self.assertTrue(any("[ready]" in content for content in sent))
            self.assertIn("ask_stream_no_final_error target=thread-1", log_text)
        finally:
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_ask_stream_no_final_is_suppressed_after_steering_handoff(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait
                activate_steering_handoff(target_thread_id)
                return 0, "[delivery_verified] taxlab:1"

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
            self.assertIn("ask_stream_suppressed_after_steering target=thread-1", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_ask_stream_commentary_is_sent_after_steering_handoff(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()
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
                activate_steering_handoff(target_thread_id)
                relay.feed_line("[commentary]")
                relay.feed_line("checking live relay")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[commentary]\nchecking live relay\n\n[ready]"

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

            sent = [content for content, _view in message.channel.messages]
            self.assertEqual(sent, ["In progress\n\nchecking live relay"])
            self.assertIn("ask_stream_done exit=0 target=thread-1 sent_live=True final=False", log_text)
            self.assertIn("ask_stream_suppressed_after_steering target=thread-1 sent_live=True", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_ask_stream_final_is_suppressed_after_steering_handoff(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()
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
                activate_steering_handoff(target_thread_id)
                relay.feed_line("[final_answer]")
                relay.feed_line("original final")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\noriginal final\n\n[ready]"

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
            self.assertIn("ask_stream_suppressed_after_steering target=thread-1", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream


if __name__ == "__main__":
    _ = unittest.main()

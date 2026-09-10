from __future__ import annotations

from pathlib import Path
import asyncio  # noqa: ANYIO_OK
import tempfile
import threading
import unittest

import codex_discord_bot as bot
import codex_discord_prompt_busy_result as prompt_busy_result

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class DiscordRunPromptDeliveryLockIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_run_prompt_and_send_holds_same_target_lock_during_busy_mirror_wait(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        original_snapshot = bot.snapshot_ask_prompt_delivery_state
        original_wait_settle = bot.wait_for_mirrored_busy_delegation_settle
        old_ask_locks = bot.get_runtime_state().ask_delivery_locks
        first_started = threading.Event()
        second_started = threading.Event()
        release_settle = asyncio.Event()
        calls: list[str] = []
        try:
            bot.get_runtime_state().ask_delivery_locks = {}
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, target_thread_id or "selected")
            bot.should_delegate_output_to_session_mirror = lambda channel, target_thread_id: True
            bot.snapshot_ask_prompt_delivery_state = lambda target_thread_id: (None, {})

            async def fake_wait_settle(
                prompt: str,
                *,
                target_thread_id: str | None = None,
                recent_offsets: prompt_busy_result.RecentOffsets | None = None,
            ) -> None:
                _ = prompt, target_thread_id, recent_offsets
                _ = await release_settle.wait()

            bot.wait_for_mirrored_busy_delegation_settle = fake_wait_settle

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = relay, force_while_busy, wait, target_thread_id
                calls.append(prompt)
                if prompt == "first":
                    first_started.set()
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
                second_started.set()
                relay.feed_line("[final_answer]")
                relay.feed_line("second done")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\nsecond done\n\n[ready]"

            bot.run_ask_stream = fake_run_ask_stream
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_ASK_BUSY_RETRY_ATTEMPTS", "0"):
                        task_a = asyncio.ensure_future(
                            run_prompt_and_send()(
                                FakeTarget(),
                                "first",
                                ack_sent=True,
                                target_thread_id="thread-a",
                            )
                        )
                        self.assertTrue(await asyncio.to_thread(first_started.wait, 1))
                        task_b = asyncio.ensure_future(
                            run_prompt_and_send()(
                                FakeTarget(),
                                "second",
                                ack_sent=True,
                                target_thread_id="thread-a",
                            )
                        )
                        await asyncio.sleep(0.1)
                        self.assertFalse(second_started.is_set())
                        release_settle.set()
                        self.assertTrue(await asyncio.to_thread(second_started.wait, 1))
                        _ = await asyncio.wait_for(asyncio.gather(task_a, task_b), timeout=2)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(calls, ["first", "second"])
            self.assertIn("ask_delivery_wait target=thread-a", log_text)
            self.assertIn("ask_delivery_wait_done target=thread-a", log_text)
        finally:
            release_settle.set()
            bot.get_runtime_state().ask_delivery_locks = old_ask_locks
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream
            bot.should_delegate_output_to_session_mirror = original_should_delegate
            bot.snapshot_ask_prompt_delivery_state = original_snapshot
            bot.wait_for_mirrored_busy_delegation_settle = original_wait_settle


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from pathlib import Path
import asyncio  # noqa: ANYIO_OK
import tempfile
import threading
import unittest

import codex_discord_bot as bot

from tests.test_codex_discord_ask_busy_failure_integration import run_prompt_and_send
from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class DiscordRunPromptSerializationIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_run_prompt_and_send_starts_different_active_app_target_without_queueing(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_condition = bot.get_runtime_state().codex_app_turn_condition
        old_active_key = bot.get_runtime_state().codex_app_active_target_key
        old_active_count = bot.get_runtime_state().codex_app_active_target_count
        old_ask_locks = bot.get_runtime_state().ask_delivery_locks
        first_started = threading.Event()
        release_first = threading.Event()
        second_started = threading.Event()
        calls: list[tuple[str, str | None]] = []
        try:
            bot.get_runtime_state().codex_app_turn_condition = None
            bot.get_runtime_state().codex_app_active_target_key = None
            bot.get_runtime_state().codex_app_active_target_count = 0
            bot.get_runtime_state().ask_delivery_locks = {}
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, target_thread_id or "selected")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, force_while_busy, wait
                calls.append(("ask", target_thread_id))
                if target_thread_id == "thread-a":
                    first_started.set()
                    _ = release_first.wait(2)
                if target_thread_id == "thread-b":
                    second_started.set()
                relay.feed_line("[final_answer]")
                relay.feed_line(f"done {target_thread_id}")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, f"[final_answer]\ndone {target_thread_id}\n\n[ready]"

            bot.run_ask_stream = fake_run_ask_stream

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    target_a = FakeTarget()
                    target_b = FakeTarget()
                    task_a = asyncio.ensure_future(
                        run_prompt_and_send()(
                            target_a,
                            "first",
                            ack_sent=True,
                            target_thread_id="thread-a",
                        )
                    )
                    self.assertTrue(await asyncio.to_thread(first_started.wait, 1))
                    task_b = asyncio.ensure_future(
                        run_prompt_and_send()(
                            target_b,
                            "second",
                            ack_sent=True,
                            target_thread_id="thread-b",
                        )
                    )
                    self.assertTrue(await asyncio.to_thread(second_started.wait, 1))
                    release_first.set()
                    _ = await asyncio.wait_for(asyncio.gather(task_a, task_b), timeout=3)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(calls, [("ask", "thread-a"), ("ask", "thread-b")])
            self.assertEqual(target_a.messages, [("Final\n\ndone thread-a", None)])
            self.assertEqual(target_b.messages, [("Final\n\ndone thread-b", None)])
            self.assertNotIn("codex_app_turn_wait target=thread-b active=thread-a", log_text)
            self.assertNotIn("ask_after_cross_session_wait_done exit=0 target=thread-b", log_text)
        finally:
            release_first.set()
            bot.get_runtime_state().codex_app_turn_condition = old_condition
            bot.get_runtime_state().codex_app_active_target_key = old_active_key
            bot.get_runtime_state().codex_app_active_target_count = old_active_count
            bot.get_runtime_state().ask_delivery_locks = old_ask_locks
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream

    async def test_run_prompt_and_send_serializes_same_active_app_target(self) -> None:
        original_resolve_target_ref = bot.resolve_target_ref
        original_run_ask_stream = bot.run_ask_stream
        old_condition = bot.get_runtime_state().codex_app_turn_condition
        old_active_key = bot.get_runtime_state().codex_app_active_target_key
        old_active_count = bot.get_runtime_state().codex_app_active_target_count
        old_ask_locks = bot.get_runtime_state().ask_delivery_locks
        first_started = threading.Event()
        second_started = threading.Event()
        release_first = threading.Event()
        calls: list[str | None] = []
        try:
            bot.get_runtime_state().codex_app_turn_condition = None
            bot.get_runtime_state().codex_app_active_target_key = None
            bot.get_runtime_state().codex_app_active_target_count = 0
            bot.get_runtime_state().ask_delivery_locks = {}
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, target_thread_id or "selected")

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = force_while_busy, wait, target_thread_id
                calls.append(target_thread_id)
                if prompt == "first":
                    first_started.set()
                    _ = release_first.wait(2)
                if prompt == "second":
                    second_started.set()
                relay.feed_line("[final_answer]")
                relay.feed_line(f"done {prompt}")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, f"[final_answer]\ndone {prompt}\n\n[ready]"

            bot.run_ask_stream = fake_run_ask_stream
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
            release_first.set()
            self.assertTrue(await asyncio.to_thread(second_started.wait, 1))
            _ = await asyncio.wait_for(asyncio.gather(task_a, task_b), timeout=3)

            self.assertEqual(calls, ["thread-a", "thread-a"])
        finally:
            release_first.set()
            bot.get_runtime_state().codex_app_turn_condition = old_condition
            bot.get_runtime_state().codex_app_active_target_key = old_active_key
            bot.get_runtime_state().codex_app_active_target_count = old_active_count
            bot.get_runtime_state().ask_delivery_locks = old_ask_locks
            bot.resolve_target_ref = original_resolve_target_ref
            bot.run_ask_stream = original_run_ask_stream


if __name__ == "__main__":
    _ = unittest.main()

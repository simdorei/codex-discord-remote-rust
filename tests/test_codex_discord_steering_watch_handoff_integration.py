from __future__ import annotations

# pyright: reportUnknownMemberType=false

import asyncio  # noqa: ANYIO_OK
from pathlib import Path
from typing import Protocol
import tempfile
import time
import unittest

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime
import codex_discord_steering_watch as steering_watch

from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class WatchRelay(Protocol):
    def feed_line(self, line: str) -> None:
        ...

    def finish(self) -> None:
        ...


class DiscordSteeringWatchHandoffIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_post_approval_watch_streams_final_after_steering_handoff(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()

            def fake_run_watch(
                watch_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = watch_result, timeout_sec
                _ = bot.mark_steering_handoff("thread-1")
                relay.feed_line("[final_answer]")
                relay.feed_line("approved follow-up")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\napproved follow-up\n\n[ready]"

            bot.run_steering_watch_stream = fake_run_watch
            target = FakeTarget()
            watch_result = bot.SteeringPromptResult(
                0,
                "[approval_submitted]",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    streamed = await bot.stream_post_approval_result_to_channel(
                        target,
                        watch_result,
                        "thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(streamed)
            self.assertEqual(target.messages, [("Final\n\napproved follow-up", None)])
            self.assertIn("approval_followup_watch_done exit=0 target=thread-1", log_text)
            self.assertNotIn("discord_relay_suppressed_after_steering", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.run_steering_watch_stream = original_run_watch

    async def test_steering_watch_does_not_suppress_handoff_before_watch_start(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()
            _ = bot.mark_steering_handoff("thread-1")

            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.feed_line("[final_answer]")
                relay.feed_line("current steered final")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\ncurrent steered final\n\n[ready]"

            bot.run_steering_watch_stream = fake_run_watch
            target = FakeTarget()
            steering_result = bot.SteeringPromptResult(
                0,
                "[delivery_verified]",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    _ = await bot.stream_steering_prompt_result_to_channel(
                        target,
                        steering_result,
                        "thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(target.messages, [("Final\n\ncurrent steered final", None)])
            self.assertNotIn("steer_watch_suppressed_after_newer_handoff", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.run_steering_watch_stream = original_run_watch

    async def test_older_steering_watch_suppresses_after_newer_handoff(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        old_handoffs = dict(bot.get_runtime_state().steering_handoffs)
        try:
            bot.get_runtime_state().steering_handoffs.clear()

            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                key = discord_runtime.normalize_runner_key("thread-1")
                bot.get_runtime_state().steering_handoffs[key] = time.monotonic() + 1.0
                relay.feed_line("[final_answer]")
                relay.feed_line("duplicate final")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\nduplicate final\n\n[ready]"

            bot.run_steering_watch_stream = fake_run_watch
            target = FakeTarget()
            steering_result = bot.SteeringPromptResult(
                0,
                "[delivery_verified]",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    _ = await bot.stream_steering_prompt_result_to_channel(
                        target,
                        steering_result,
                        "thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(target.messages, [])
            self.assertIn("steer_watch_suppressed_after_newer_handoff target=thread-1", log_text)
        finally:
            bot.get_runtime_state().steering_handoffs.clear()
            bot.get_runtime_state().steering_handoffs.update(old_handoffs)
            bot.run_steering_watch_stream = original_run_watch

    async def test_older_relay_suppresses_after_newer_relay_for_same_thread(self) -> None:
        old_generations = dict(bot.get_runtime_state().active_discord_relay_generations)
        try:
            bot.get_runtime_state().active_discord_relay_generations.clear()
            target = FakeTarget()
            loop = asyncio.get_running_loop()
            older = bot.DiscordAskRelay(loop, target, "thread-1", "project:1")
            _ = bot.DiscordAskRelay(loop, target, "thread-1", "project:1")

            older.feed_line("[final_answer]")
            older.feed_line("stale final")
            older.feed_line("[ready]")
            older.finish()

            self.assertEqual(target.messages, [])
            self.assertTrue(older.suppressed_after_steering)
        finally:
            bot.get_runtime_state().active_discord_relay_generations.clear()
            bot.get_runtime_state().active_discord_relay_generations.update(old_generations)


if __name__ == "__main__":
    _ = unittest.main()

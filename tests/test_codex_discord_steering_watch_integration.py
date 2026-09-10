from __future__ import annotations

# pyright: reportUnknownMemberType=false

from pathlib import Path
from typing import Protocol
import tempfile
import unittest

import codex_discord_bot as bot
import codex_discord_steering_watch as steering_watch

from tests.test_codex_discord_bot import EnvPatch, FakeTarget


class WatchRelay(Protocol):
    def feed_line(self, line: str) -> None:
        ...

    def finish(self) -> None:
        ...


class DiscordSteeringWatchIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_steering_watch_uses_finite_timeout(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        timeouts: list[float] = []
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result
                timeouts.append(timeout_sec)
                relay.finish()
                return 2, ""

            bot.run_steering_watch_stream = fake_run_watch
            target = FakeTarget()
            steering_result = bot.SteeringPromptResult(
                0,
                "[delivery_pending]",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
                delivery_pending=True,
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

            self.assertEqual(timeouts, [bot.STEERING_PENDING_WATCH_TIMEOUT_SECONDS])
            self.assertEqual(target.messages, [])
            self.assertIn("steer_watch_empty_failure_suppressed target=thread-1", log_text)

            target.messages.clear()
            settled_steering_result = bot.SteeringPromptResult(
                steering_result.exit_code,
                steering_result.output,
                target_thread_id=steering_result.target_thread_id,
                target_ref=steering_result.target_ref,
                session_path=steering_result.session_path,
                start_offset=steering_result.start_offset,
                delivery_pending=False,
            )
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    _ = await bot.stream_steering_prompt_result_to_channel(
                        target,
                        settled_steering_result,
                        "thread-1",
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(
                timeouts,
                [
                    bot.STEERING_PENDING_WATCH_TIMEOUT_SECONDS,
                    bot.STEERING_PENDING_WATCH_TIMEOUT_SECONDS,
                ],
            )
            self.assertEqual(target.messages, [])
            self.assertIn("steer_watch_empty_failure_suppressed target=thread-1", log_text)
        finally:
            bot.run_steering_watch_stream = original_run_watch

    async def test_steering_watch_suppresses_empty_success(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.finish()
                return 0, ""

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
            self.assertIn("steer_watch_empty_success_suppressed target=thread-1", log_text)
        finally:
            bot.run_steering_watch_stream = original_run_watch

    async def test_steering_watch_reports_nonempty_failure(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.finish()
                return 2, "watch failed with details"

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

            _ = await bot.stream_steering_prompt_result_to_channel(
                target,
                steering_result,
                "thread-1",
            )

            self.assertEqual(len(target.messages), 1)
            self.assertIn("Steering watch failed (exit 2)", target.messages[0][0])
            self.assertIn("watch failed with details", target.messages[0][0])
        finally:
            bot.run_steering_watch_stream = original_run_watch

    async def test_steering_watch_live_final_does_not_send_done_copy(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.feed_line("[final_answer]")
                relay.feed_line("steered final")
                relay.feed_line("[ready]")
                relay.finish()
                return 0, "[final_answer]\nsteered final\n\n[ready]"

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

            _ = await bot.stream_steering_prompt_result_to_channel(
                target,
                steering_result,
                "thread-1",
            )

            self.assertEqual(target.messages, [("Final\n\nsteered final", None)])
        finally:
            bot.run_steering_watch_stream = original_run_watch


if __name__ == "__main__":
    _ = unittest.main()

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


class DiscordSteeringWatchTimeoutIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_steering_watch_reports_timeout_failure(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.feed_line("[timeout]")
                relay.finish()
                return 2, "[timeout]\nCodex is still working."

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

            self.assertEqual(len(target.messages), 1)
            self.assertIn("Steering is still running in Codex.", target.messages[0][0])
            self.assertIn("Do not resend", target.messages[0][0])
            self.assertIn("steer_watch_timeout_reported target=thread-1", log_text)
        finally:
            bot.run_steering_watch_stream = original_run_watch

    async def test_steering_watch_reports_timeout_when_session_mirror_delegated(self) -> None:
        original_run_watch = bot.run_steering_watch_stream
        try:
            def fake_run_watch(
                steering_result: steering_watch.SteeringWatchResult,
                relay: WatchRelay,
                *,
                timeout_sec: float = 0,
            ) -> tuple[int, str]:
                _ = steering_result, timeout_sec
                relay.feed_line("[timeout]")
                relay.finish()
                return 2, "[timeout]\nCodex is still working."

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
                        send_commentary_blocks=False,
                        send_final_blocks=False,
                    )
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(target.messages), 1)
            self.assertIn("Steering is still running in Codex.", target.messages[0][0])
            self.assertIn("steer_watch_timeout_reported target=thread-1", log_text)
            self.assertNotIn("steer_watch_public_output_delegated_to_session_mirror", log_text)
        finally:
            bot.run_steering_watch_stream = original_run_watch


if __name__ == "__main__":
    _ = unittest.main()

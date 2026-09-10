from __future__ import annotations

from pathlib import Path
import tempfile
from typing import Protocol, cast, runtime_checkable
import unittest

import discord

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeInteraction, FakeTarget
from tests.test_codex_discord_busy_choice_steer_callback_integration import (
    BusyMessage,
    SendText,
    StreamCall,
    StreamSteering,
)


@runtime_checkable
class MirrorSteerButton(Protocol):
    @property
    def label(self) -> str | None: ...

    async def callback(self, interaction: FakeInteraction, /) -> object: ...


def find_steer_button(view: discord.ui.View) -> MirrorSteerButton:
    for item in view.children:
        if isinstance(item, MirrorSteerButton) and item.label == "Steer now":
            return item
    raise StopIteration("Steer now button not found")


class DiscordBusyChoiceSteerMirrorIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_mapped_steer_now_uses_session_mirror_output_when_archive_recommended(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_stream_steering = cast(StreamSteering, bot.stream_steering_prompt_result_to_channel)
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        original_prime_cursor = bot.prime_session_mirror_cursor_for_target
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        calls: list[StreamCall] = []
        order: list[tuple[str, str | None]] = []
        try:
            steering_result = bot.SteeringPromptResult(
                0,
                "target_thread: thread-1\n[delivery_verified] taxlab:1",
                target_thread_id="thread-1",
                target_ref="taxlab:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1" if channel_id == 222 else None

            def fake_prime_cursor(target_thread_id: str | None) -> int:
                order.append(("prime", target_thread_id))
                return 100

            def fake_run_steering_prompt(prompt: str, target_thread_id: str | None) -> bot.SteeringPromptResult:
                _ = prompt
                order.append(("run", target_thread_id))
                return steering_result

            async def fake_stream_steering(
                channel: FakeTarget,
                result: bot.SteeringPromptResult,
                target_thread_id: str | None,
                **kwargs: bool | None,
            ) -> bool:
                calls.append((channel, result, target_thread_id, dict(kwargs)))
                if kwargs.get("send_final_blocks", True):
                    await cast(SendText, channel.send)("steered final")
                return True

            def should_not_delegate(channel: FakeTarget, target_thread_id: str | None) -> bool:
                _ = channel, target_thread_id
                return False

            bot.run_steering_prompt = fake_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = fake_stream_steering
            bot.should_delegate_output_to_session_mirror = should_not_delegate
            bot.prime_session_mirror_cursor_for_target = fake_prime_cursor

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction()

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_SESSION_MIRROR", "1"):
                        _ = await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(order, [("prime", "thread-1"), ("run", "thread-1")])
            self.assertEqual(
                calls,
                [
                    (
                        message.channel,
                        steering_result,
                        "thread-1",
                        {"send_commentary_blocks": False, "send_final_blocks": False},
                    )
                ],
            )
            self.assertEqual(message.channel.messages, [("Discord steering submitted.\nmessage: please steer", None)])
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertIn("steer_now_delegated_to_session_mirror target=thread-1", log_text)
            self.assertNotIn("steered final", "\n".join(content for content, _view in message.channel.messages))
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.run_steering_prompt = original_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = original_stream_steering
            bot.should_delegate_output_to_session_mirror = original_should_delegate
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            bot.get_mirrored_codex_thread_id = original_get_mirrored


if __name__ == "__main__":
    _ = unittest.main()

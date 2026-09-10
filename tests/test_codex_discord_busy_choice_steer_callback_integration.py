from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast, runtime_checkable
import tempfile
import unittest

import discord

import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeInteraction, FakeTarget


SteeringKwargs = dict[str, bool | None]
StreamCall = tuple[FakeTarget, bot.SteeringPromptResult, str | None, SteeringKwargs]


class BusyAuthor:
    def __init__(self) -> None:
        self.id: int = 242286902982606848
        self.bot: bool = False


class BusyMessage:
    def __init__(self) -> None:
        self.channel: FakeTarget = FakeTarget()
        self.author: BusyAuthor = BusyAuthor()


@runtime_checkable
class SteerButton(Protocol):
    @property
    def label(self) -> str | None: ...

    async def callback(self, interaction: FakeInteraction, /) -> None:
        ...


def find_steer_button(view: discord.ui.View) -> SteerButton:
    for item in view.children:
        if isinstance(item, SteerButton) and item.label == "Steer now":
            return item
    raise StopIteration("Steer now button not found")


class StreamSteering(Protocol):
    def __call__(
        self,
        channel: FakeTarget,
        steering_result: bot.SteeringPromptResult | None,
        target_thread_id: str | None,
        *,
        label: str = "Steering",
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> Awaitable[bool]:
        ...


class SendText(Protocol):
    def __call__(self, content: str) -> Awaitable[None]:
        ...


class DiscordBusyChoiceSteerCallbackIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_steer_now_success_attaches_watch(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_stream_steering = cast(StreamSteering, bot.stream_steering_prompt_result_to_channel)
        original_should_delegate = bot.should_delegate_output_to_session_mirror
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
                await cast(SendText, channel.send)("steered final")
                return True

            def should_not_delegate(channel: FakeTarget, target_thread_id: str | None) -> bool:
                _ = channel, target_thread_id
                return False

            bot.run_steering_prompt = fake_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = fake_stream_steering
            bot.should_delegate_output_to_session_mirror = should_not_delegate

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction()

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True, "ephemeral": True}])
            self.assertEqual(len(interaction.followup.messages), 1)
            self.assertEqual(interaction.followup.kwargs[0], {"ephemeral": True})
            self.assertIn("Steering sent", str(interaction.followup.messages[0]))
            self.assertEqual(
                calls,
                [
                    (
                        message.channel,
                        steering_result,
                        "thread-1",
                        {"send_commentary_blocks": None, "send_final_blocks": True},
                    )
                ],
            )
            self.assertEqual(
                message.channel.messages,
                [
                    ("Discord steering submitted.\nmessage: please steer", None),
                    ("steered final", None),
                ],
            )
            self.assertIn("steering_start_ack_sent target=thread-1", log_text)
            self.assertIn("steer_now_sent exit=0 target=thread-1", log_text)
        finally:
            bot.run_steering_prompt = original_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = original_stream_steering
            bot.should_delegate_output_to_session_mirror = original_should_delegate

    async def test_steer_now_delegates_public_output_to_session_mirror(self) -> None:
        original_run_steering_prompt = bot.run_steering_prompt
        original_stream_steering = cast(StreamSteering, bot.stream_steering_prompt_result_to_channel)
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        original_prime_cursor = bot.prime_session_mirror_cursor_for_target
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

            def fake_prime_cursor(target_thread_id: str | None) -> int:
                order.append(("prime", target_thread_id))
                return 0

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

            def should_delegate(channel: FakeTarget, target_thread_id: str | None) -> bool:
                _ = channel, target_thread_id
                return True

            bot.run_steering_prompt = fake_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = fake_stream_steering
            bot.should_delegate_output_to_session_mirror = should_delegate
            bot.prime_session_mirror_cursor_for_target = fake_prime_cursor

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = BusyMessage()
                view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
                button = find_steer_button(view)
                interaction = FakeInteraction()

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertTrue(interaction.response.deferred)
            self.assertEqual(len(interaction.followup.messages), 1)
            self.assertIn("Steering sent", str(interaction.followup.messages[0]))
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
            self.assertEqual(
                message.channel.messages,
                [("Discord steering submitted.\nmessage: please steer", None)],
            )
            self.assertIn("steer_now_delegated_to_session_mirror target=thread-1", log_text)
        finally:
            bot.run_steering_prompt = original_run_steering_prompt
            bot.stream_steering_prompt_result_to_channel = original_stream_steering
            bot.should_delegate_output_to_session_mirror = original_should_delegate
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor


if __name__ == "__main__":
    _ = unittest.main()

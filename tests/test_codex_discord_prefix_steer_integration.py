from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from typing import Protocol, cast

import codex_desktop_bridge as bridge
import codex_discord_bot as bot

from tests.test_codex_discord_bot import EnvPatch, FakeMessage


class MessageableLike(Protocol):
    async def send(self, content: str, view: bot.BusyChoiceView | None = None) -> None:
        ...


class HandlePrefixCommand(Protocol):
    def __call__(
        self,
        client: SimpleNamespace,
        message: FakeMessage,
        command_line: str,
    ) -> Awaitable[None]:
        ...


class RunSteeringPrompt(Protocol):
    def __call__(self, prompt: str, target_thread_id: str | None) -> bot.SteeringPromptResult:
        ...


class StreamSteeringPromptResult(Protocol):
    def __call__(
        self,
        channel: MessageableLike,
        steering_result: bot.SteeringPromptResult | None,
        target_thread_id: str | None,
        *,
        label: str = "Steering",
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> Awaitable[bool]:
        ...


class PrimeSessionMirrorCursor(Protocol):
    def __call__(self, target_thread_id: str | None) -> int:
        ...


class ChooseThread(Protocol):
    def __call__(self, thread_id: str, cwd: str | None = None) -> SimpleNamespace:
        ...


class GetThreadContextUsage(Protocol):
    def __call__(self, thread: SimpleNamespace) -> None:
        ...


class ShouldRecommendArchive(Protocol):
    def __call__(self, thread: SimpleNamespace, context_usage: None) -> bool:
        ...


class DiscordPrefixSteerIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_prefix_steer_sends_prompt_without_button_click(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_run_steering = cast(RunSteeringPrompt, bot.run_steering_prompt)
        original_stream = cast(StreamSteeringPromptResult, bot.stream_steering_prompt_result_to_channel)
        original_prime_cursor = cast(PrimeSessionMirrorCursor, bot.prime_session_mirror_cursor_for_target)
        original_choose_thread = cast(ChooseThread, getattr(bridge, "choose_thread"))
        original_get_context_usage = cast(GetThreadContextUsage, getattr(bridge, "get_thread_context_usage"))
        original_should_recommend_archive = cast(ShouldRecommendArchive, getattr(bridge, "should_recommend_archive"))
        try:
            def fake_choose_thread(thread_id: str, cwd: str | None = None) -> SimpleNamespace:
                _ = cwd
                return SimpleNamespace(id=thread_id, tokens_used=0)

            def fake_get_context_usage(thread: SimpleNamespace) -> None:
                _ = thread

            def fake_should_recommend_archive(thread: SimpleNamespace, context_usage: None) -> bool:
                _ = thread, context_usage
                return False

            bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1"
            setattr(bridge, "choose_thread", fake_choose_thread)
            setattr(bridge, "get_thread_context_usage", fake_get_context_usage)
            setattr(bridge, "should_recommend_archive", fake_should_recommend_archive)
            observed: list[tuple[str, str | None]] = []
            order: list[tuple[str, str | None]] = []
            streamed: list[tuple[bot.SteeringPromptResult | None, str | None, bool | None, bool]] = []

            def fake_prime_cursor(target_thread_id: str | None) -> int:
                order.append(("prime", target_thread_id))
                return 0

            def fake_run_steering(prompt: str, target_thread_id: str | None) -> bot.SteeringPromptResult:
                order.append(("run", target_thread_id))
                observed.append((prompt, target_thread_id))
                return bot.SteeringPromptResult(
                    0,
                    "[qa_delivery_verified]",
                    target_thread_id=target_thread_id,
                    target_ref=target_thread_id or "-",
                    session_path="qa-session.jsonl",
                    start_offset=0,
                )

            async def fake_stream(
                channel: MessageableLike,
                steering_result: bot.SteeringPromptResult | None,
                target_thread_id: str | None,
                *,
                label: str = "Steering",
                send_commentary_blocks: bool | None = None,
                send_final_blocks: bool = True,
            ) -> bool:
                _ = channel, label
                streamed.append(
                    (steering_result, target_thread_id, send_commentary_blocks, send_final_blocks)
                )
                return True

            bot.run_steering_prompt = fake_run_steering
            bot.stream_steering_prompt_result_to_channel = fake_stream
            bot.prime_session_mirror_cursor_for_target = fake_prime_cursor
            message = FakeMessage()
            handle_prefix = cast(HandlePrefixCommand, bot.handle_prefix_command)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("DISCORD_ENABLE_QA_COMMANDS", "1"):
                        await handle_prefix(
                            SimpleNamespace(),
                            message,
                            "steer please steer now",
                        )

        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.run_steering_prompt = original_run_steering
            bot.stream_steering_prompt_result_to_channel = original_stream
            bot.prime_session_mirror_cursor_for_target = original_prime_cursor
            setattr(bridge, "choose_thread", original_choose_thread)
            setattr(bridge, "get_thread_context_usage", original_get_context_usage)
            setattr(bridge, "should_recommend_archive", original_should_recommend_archive)

        self.assertEqual(observed, [("please steer now", "thread-1")])
        self.assertEqual(order, [("prime", "thread-1"), ("run", "thread-1")])
        self.assertEqual(len(streamed), 1)
        self.assertEqual(streamed[0][1], "thread-1")
        self.assertEqual(streamed[0][2:], (False, False))
        self.assertEqual(len(message.channel.messages), 1)
        self.assertEqual(message.channel.messages[0][0], "Steering sent\n\n[qa_delivery_verified]")

    async def test_prefix_steer_is_disabled_by_default(self) -> None:
        message = FakeMessage()
        handle_prefix = cast(HandlePrefixCommand, bot.handle_prefix_command)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await handle_prefix(SimpleNamespace(), message, "steer please steer now")

        self.assertEqual(
            message.channel.messages,
            [("Discord QA steering is disabled. Set DISCORD_ENABLE_QA_COMMANDS=1 to enable it.", None)],
        )


if __name__ == "__main__":
    _ = unittest.main()

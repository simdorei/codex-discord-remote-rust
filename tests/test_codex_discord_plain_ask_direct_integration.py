from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_bot import EnvPatch, FakeMessage, FakeTarget


PromptFlowCall = tuple[str, str | None, FakeMessage | None]


class RunPromptFlow(Protocol):
    def __call__(
        self,
        channel: FakeTarget,
        prompt: str,
        *,
        queued: bool = False,
        source_message: FakeMessage | None = None,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]:
        ...


class HandlePlainAsk(Protocol):
    def __call__(
        self,
        message: FakeMessage,
        prompt: str,
        *,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]:
        ...


def run_prompt_flow() -> RunPromptFlow:
    return cast(RunPromptFlow, getattr(bot, "run_prompt_flow"))


def handle_plain_ask() -> HandlePlainAsk:
    return cast(HandlePlainAsk, getattr(bot, "handle_plain_ask"))


def empty_interactive_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
    _ = target_thread_id
    return "", None, ""


def fail_busy_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
    _ = target_thread_id
    raise AssertionError("plain ask should not inspect busy state before sending")


def no_recent_prompt(target_thread_id: str | None, prompt: str) -> bool:
    _ = target_thread_id, prompt
    return False


class DiscordPlainAskDirectIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_plain_ask_does_not_check_busy_before_prompt_flow(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_get_busy_state = bot.get_busy_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        calls: list[PromptFlowCall] = []
        try:
            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                queued: bool = False,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel, queued
                calls.append((prompt, target_thread_id, source_message))

            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.get_busy_state_for_thread = fail_busy_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.run_prompt_flow = fake_run_prompt_flow
            message = FakeMessage()

            await handle_plain_ask()(message, "please send", target_thread_id="thread-1")

            self.assertEqual(message.channel.messages, [])
            self.assertEqual(calls, [("please send", "thread-1", message)])
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.get_busy_state_for_thread = original_get_busy_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow

    async def test_plain_ask_idle_runner_check_does_not_create_queue_state(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        target_key = discord_runtime.normalize_runner_key("thread-1")
        calls: list[tuple[str, str | None]] = []
        try:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)

            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                queued: bool = False,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel, queued, source_message
                calls.append((prompt, target_thread_id))

            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.run_prompt_flow = fake_run_prompt_flow

            await handle_plain_ask()(FakeMessage(), "first request", target_thread_id="thread-1")

            self.assertEqual(calls, [("first request", "thread-1")])
            async with bot.THREAD_RUNNERS_LOCK:
                self.assertNotIn(target_key, bot.THREAD_RUNNERS)
        finally:
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow

    async def test_plain_ask_skips_recent_codex_app_duplicate(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        try:
            async def fail_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                queued: bool = False,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel, prompt, queued, source_message, target_thread_id
                raise AssertionError("duplicate app prompts must not be sent to Codex again")

            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = lambda target_thread_id, prompt: True
            bot.run_prompt_flow = fail_run_prompt_flow
            message = FakeMessage()

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await handle_plain_ask()(message, "same prompt", target_thread_id="thread-1")
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(
                message.channel.messages,
                [("Already in Codex app. Skipping duplicate Discord delivery for this mapped thread.", None)],
            )
            self.assertEqual(bot.get_runtime_state().recent_discord_origin_prompts, {})
            self.assertIn("plain_ask_duplicate_recent_app_prompt_skipped target=thread-1", log_text)
        finally:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from collections.abc import Awaitable
from pathlib import Path
from typing import Protocol, cast
import tempfile
import unittest

import codex_discord_bot as bot

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


class DiscordPlainAskPreflightIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_stale_busy_plain_ask_still_sends_without_preflight(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_get_busy_state = bot.get_busy_state_for_thread
        original_get_stale_info = bot.get_stale_busy_steer_block_info
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        calls: list[PromptFlowCall] = []
        try:
            def stale_info(target_thread_id: str | None) -> tuple[str, str, float]:
                _ = target_thread_id
                return "thread-1", "taxlab:1", 660.0

            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel
                calls.append((prompt, target_thread_id, source_message))

            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.get_busy_state_for_thread = fail_busy_state
            bot.get_stale_busy_steer_block_info = stale_info
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.run_prompt_flow = fake_run_prompt_flow
            message = FakeMessage()

            await handle_plain_ask()(message, "please steer", target_thread_id="thread-1")

            self.assertEqual(message.channel.messages, [])
            self.assertEqual(calls, [("please steer", "thread-1", message)])
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.get_busy_state_for_thread = original_get_busy_state
            bot.get_stale_busy_steer_block_info = original_get_stale_info
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow

    async def test_other_thread_busy_plain_ask_still_enters_target_ask_flow(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_get_busy_state = bot.get_busy_state_for_thread
        original_is_thread_runner_busy = bot.is_thread_runner_busy
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        calls: list[PromptFlowCall] = []
        try:
            async def runner_idle(target_thread_id: str | None) -> bool:
                _ = target_thread_id
                return False

            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel
                calls.append((prompt, target_thread_id, source_message))

            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                _ = session_path.write_text("", encoding="utf-8")
                bot.get_interactive_state_for_thread = empty_interactive_state
                bot.get_busy_state_for_thread = fail_busy_state
                bot.has_recent_codex_app_user_prompt = no_recent_prompt
                bot.is_thread_runner_busy = runner_idle
                bot.run_prompt_flow = fake_run_prompt_flow
                log_path = Path(temp_dir) / "discord-smoke.log"
                message = FakeMessage()
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await handle_plain_ask()(message, "please run", target_thread_id="thread-1")
                log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

            self.assertEqual(message.channel.messages, [])
            self.assertEqual(calls, [("please run", "thread-1", message)])
            self.assertNotIn("busy_preflight", log_text)
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.get_busy_state_for_thread = original_get_busy_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.is_thread_runner_busy = original_is_thread_runner_busy
            bot.run_prompt_flow = original_run_prompt_flow

    async def test_idle_target_plain_ask_delegates_to_prompt_flow(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_get_busy_state = bot.get_busy_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        calls: list[PromptFlowCall] = []
        try:
            def idle_busy_state(target_thread_id: str | None) -> tuple[str, str | None, str]:
                _ = target_thread_id
                return "idle", "thread-1", "taxlab:1"

            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel
                calls.append((prompt, target_thread_id, source_message))

            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.get_busy_state_for_thread = idle_busy_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.run_prompt_flow = fake_run_prompt_flow
            message = FakeMessage()

            await handle_plain_ask()(message, "please queue", target_thread_id="thread-1")

            self.assertEqual(message.channel.messages, [])
            self.assertEqual(calls, [("please queue", "thread-1", message)])
        finally:
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.get_busy_state_for_thread = original_get_busy_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow


if __name__ == "__main__":
    _ = unittest.main()

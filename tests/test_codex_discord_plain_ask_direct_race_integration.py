from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import unittest

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_bot import FakeMessage, FakeTarget
from tests.test_codex_discord_plain_ask_direct_integration import (
    empty_interactive_state,
    handle_plain_ask,
    no_recent_prompt,
    run_prompt_flow,
)


class DiscordPlainAskDirectRaceIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_plain_ask_offers_choice_before_same_thread_direct_ask_enters_stream(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_prompt_flow = run_prompt_flow()
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        target_key = discord_runtime.normalize_runner_key("thread-1")
        started = asyncio.Event()
        release = asyncio.Event()
        flow_calls: list[tuple[str, str | None]] = []
        first_task: asyncio.Future[None] | None = None
        try:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            async with bot.get_runtime_state().active_direct_ask_lock:
                bot.get_runtime_state().active_direct_ask_target_keys.discard(target_key)
            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt

            async def fake_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                queued: bool = False,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel, queued, source_message
                flow_calls.append((prompt, target_thread_id))
                if prompt == "first request":
                    _ = started.set()
                    _ = await release.wait()
                    return
                raise AssertionError("second same-thread ask must wait for a queue/steer choice")

            bot.run_prompt_flow = fake_run_prompt_flow
            channel = FakeTarget()
            first_message = FakeMessage()
            second_message = FakeMessage()
            first_message.channel = channel
            second_message.channel = channel

            first_task = asyncio.ensure_future(
                handle_plain_ask()(first_message, "first request", target_thread_id="thread-1")
            )
            _ = await asyncio.wait_for(started.wait(), timeout=1)

            await handle_plain_ask()(second_message, "second request", target_thread_id="thread-1")

            busy_messages = [
                (content, view)
                for content, view in channel.messages
                if isinstance(view, bot.BusyChoiceView)
            ]
            self.assertEqual(flow_calls, [("first request", "thread-1")])
            self.assertEqual(len(busy_messages), 1)
            _content, view = busy_messages[0]
            self.assertEqual(view.target_thread_id, "thread-1")

            _ = release.set()
            await asyncio.wait_for(first_task, timeout=1)
        finally:
            _ = release.set()
            if first_task is not None and not first_task.done():
                _ = first_task.cancel()
                try:
                    await first_task
                except asyncio.CancelledError as exc:
                    _ = exc
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            async with bot.get_runtime_state().active_direct_ask_lock:
                bot.get_runtime_state().active_direct_ask_target_keys.discard(target_key)
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_prompt_flow = original_run_prompt_flow


if __name__ == "__main__":
    _ = unittest.main()

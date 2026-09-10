from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import threading
import unittest
from typing import override

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_bot import FakeMessage, FakeTarget
from tests.test_codex_discord_plain_ask_direct_integration import (
    empty_interactive_state,
    handle_plain_ask,
    no_recent_prompt,
)


class DiscordPlainAskDeliveryLockIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_plain_ask_offers_choice_before_first_direct_ask_acquires_delivery_lock(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_resolve_target_ref = bot.resolve_target_ref
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        target_key = discord_runtime.normalize_runner_key("thread-1")
        start_send_entered = threading.Event()
        release_start_send = threading.Event()
        run_calls: list[str] = []
        first_task: asyncio.Future[None] | None = None

        class BlockingStartTarget(FakeTarget):
            @override
            async def send(self, content: str, view: bot.BusyChoiceView | None = None) -> None:
                if view is None and content.startswith("In progress\nmessage: first request"):
                    start_send_entered.set()
                    _ = await asyncio.to_thread(release_start_send.wait, 2)
                self.messages.append((content, view))

        try:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            async with bot.get_runtime_state().active_direct_ask_lock:
                bot.get_runtime_state().active_direct_ask_target_keys.discard(target_key)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.build_context_warning = lambda target_thread_id: ""
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.should_delegate_output_to_session_mirror = lambda channel, target_thread_id: False

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = relay, force_while_busy, wait, target_thread_id
                run_calls.append(prompt)
                return 0, "done"

            bot.run_ask_stream = fake_run_ask_stream
            channel = BlockingStartTarget()
            first_message = FakeMessage()
            second_message = FakeMessage()
            first_message.channel = channel
            second_message.channel = channel

            first_task = asyncio.ensure_future(
                handle_plain_ask()(first_message, "first request", target_thread_id="thread-1")
            )
            self.assertTrue(await asyncio.to_thread(start_send_entered.wait, 1))

            await handle_plain_ask()(second_message, "second request", target_thread_id="thread-1")

            busy_messages = [
                (content, view)
                for content, view in channel.messages
                if isinstance(view, bot.BusyChoiceView)
            ]
            self.assertEqual(len(busy_messages), 1)
            _content, view = busy_messages[0]
            self.assertEqual(view.target_thread_id, "thread-1")
            self.assertEqual(run_calls, [])

            release_start_send.set()
            await asyncio.wait_for(first_task, timeout=1)
            self.assertEqual(run_calls, ["first request"])
        finally:
            release_start_send.set()
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
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_ask_stream = original_run_ask_stream
            bot.build_context_warning = original_build_context_warning
            bot.resolve_target_ref = original_resolve_target_ref
            bot.should_delegate_output_to_session_mirror = original_should_delegate


if __name__ == "__main__":
    _ = unittest.main()

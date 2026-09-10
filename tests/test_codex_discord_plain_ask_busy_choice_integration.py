from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Sequence
from pathlib import Path
from typing import Protocol, cast
import tempfile
import threading
import unittest

import codex_discord_bot as bot
import codex_discord_runtime as discord_runtime

from tests.test_codex_discord_bot import EnvPatch, FakeMessage, FakeTarget
from tests.test_codex_discord_plain_ask_direct_integration import (
    empty_interactive_state,
    handle_plain_ask,
    no_recent_prompt,
    run_prompt_flow,
)


class ButtonLike(Protocol):
    label: str
    disabled: bool


class DiscordPlainAskBusyChoiceIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        original_app_server_transport_enabled = bot.app_server_transport_enabled

        def restore_app_server_transport_enabled() -> None:
            bot.app_server_transport_enabled = original_app_server_transport_enabled

        self.addCleanup(restore_app_server_transport_enabled)
        bot.app_server_transport_enabled = lambda: False

    async def test_plain_ask_offers_queue_or_steer_when_same_thread_runner_is_busy(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_is_thread_runner_busy = bot.is_thread_runner_busy
        original_run_prompt_flow = run_prompt_flow()
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        try:
            async def runner_busy(target_thread_id: str | None) -> bool:
                return target_thread_id == "thread-1"

            async def fail_run_prompt_flow(
                channel: FakeTarget,
                prompt: str,
                *,
                queued: bool = False,
                source_message: FakeMessage | None = None,
                target_thread_id: str | None = None,
            ) -> None:
                _ = channel, prompt, queued, source_message, target_thread_id
                raise AssertionError("same-thread busy ask must wait for user queue/steer choice")

            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.is_thread_runner_busy = runner_busy
            bot.run_prompt_flow = fail_run_prompt_flow
            message = FakeMessage()

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await handle_plain_ask()(message, "second request", target_thread_id="thread-1")
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(len(message.channel.messages), 1)
            content, view = message.channel.messages[0]
            self.assertIn("Choose the Discord action for this message.", content)
            self.assertIsInstance(view, bot.BusyChoiceView)
            busy_view = cast(bot.BusyChoiceView, view)
            self.assertEqual(busy_view.target_thread_id, "thread-1")
            self.assertTrue(busy_view.allow_steer)
            children = cast(Sequence[ButtonLike], busy_view.children)
            steer_button = next(item for item in children if item.label == "Steer now")
            self.assertFalse(steer_button.disabled)
            self.assertIn("busy_choice_sent reason=same_thread_runner_busy target=thread-1", log_text)
        finally:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.is_thread_runner_busy = original_is_thread_runner_busy
            bot.run_prompt_flow = original_run_prompt_flow

    async def test_plain_ask_offers_choice_while_same_thread_direct_ask_is_active(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_resolve_target_ref = bot.resolve_target_ref
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        target_key = discord_runtime.normalize_runner_key("thread-1")
        started = threading.Event()
        release = threading.Event()
        first_task: asyncio.Future[None] | None = None
        try:
            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait, target_thread_id
                started.set()
                _ = release.wait(timeout=2)
                return 0, "done"

            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            bot.get_interactive_state_for_thread = empty_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.build_context_warning = lambda target_thread_id: ""
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            bot.should_delegate_output_to_session_mirror = lambda channel, target_thread_id: False
            bot.run_ask_stream = fake_run_ask_stream
            channel = FakeTarget()
            first_message = FakeMessage()
            second_message = FakeMessage()
            first_message.channel = channel
            second_message.channel = channel

            first_task = asyncio.ensure_future(
                handle_plain_ask()(first_message, "first request", target_thread_id="thread-1")
            )
            self.assertTrue(await asyncio.to_thread(started.wait, 1))

            await handle_plain_ask()(second_message, "second request", target_thread_id="thread-1")

            busy_messages = [
                (content, view)
                for content, view in channel.messages
                if isinstance(view, bot.BusyChoiceView)
            ]
            self.assertEqual(len(busy_messages), 1)
            content, view = busy_messages[0]
            self.assertIn("Choose the Discord action for this message.", content)
            busy_view = view
            self.assertEqual(busy_view.target_thread_id, "thread-1")
            self.assertTrue(busy_view.allow_steer)
            children = cast(Sequence[ButtonLike], busy_view.children)
            labels = [item.label for item in children]
            self.assertEqual(labels, ["Steer now", "Queue next", "Stop reply", "Ignore"])
            async with bot.THREAD_RUNNERS_LOCK:
                self.assertNotIn(target_key, bot.THREAD_RUNNERS)

            release.set()
            await asyncio.wait_for(first_task, timeout=1)
        finally:
            release.set()
            if first_task is not None and not first_task.done():
                _ = first_task.cancel()
                try:
                    await first_task
                except asyncio.CancelledError as exc:
                    _ = exc
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)
            bot.get_interactive_state_for_thread = original_get_interactive_state
            bot.has_recent_codex_app_user_prompt = original_has_recent_prompt
            bot.run_ask_stream = original_run_ask_stream
            bot.build_context_warning = original_build_context_warning
            bot.resolve_target_ref = original_resolve_target_ref
            bot.should_delegate_output_to_session_mirror = original_should_delegate

    async def test_plain_ask_selected_target_uses_resolved_thread_for_busy_choice(self) -> None:
        original_get_interactive_state = bot.get_interactive_state_for_thread
        original_has_recent_prompt = bot.has_recent_codex_app_user_prompt
        original_run_ask_stream = bot.run_ask_stream
        original_build_context_warning = bot.build_context_warning
        original_resolve_target_ref = bot.resolve_target_ref
        original_should_delegate = bot.should_delegate_output_to_session_mirror
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        target_key = discord_runtime.normalize_runner_key("thread-1")
        selected_key = discord_runtime.normalize_runner_key(None)
        started = threading.Event()
        release = threading.Event()
        first_task: asyncio.Future[None] | None = None
        try:
            def selected_interactive_state(
                target_thread_id: str | None,
            ) -> tuple[str, str | None, str]:
                _ = target_thread_id
                return "", "thread-1", "taxlab:1"

            def fake_run_ask_stream(
                prompt: str,
                relay: bot.DiscordAskRelay,
                *,
                force_while_busy: bool = False,
                wait: bool = True,
                target_thread_id: str | None = None,
            ) -> tuple[int, str]:
                _ = prompt, relay, force_while_busy, wait, target_thread_id
                started.set()
                _ = release.wait(timeout=2)
                return 0, "done"

            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
                _ = bot.THREAD_RUNNERS.pop(selected_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(selected_key, None)
            bot.get_interactive_state_for_thread = selected_interactive_state
            bot.has_recent_codex_app_user_prompt = no_recent_prompt
            bot.build_context_warning = lambda target_thread_id: ""
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id or "thread-1", "taxlab:1")
            bot.should_delegate_output_to_session_mirror = lambda channel, target_thread_id: False
            bot.run_ask_stream = fake_run_ask_stream
            channel = FakeTarget()
            first_message = FakeMessage()
            second_message = FakeMessage()
            first_message.channel = channel
            second_message.channel = channel

            first_task = asyncio.ensure_future(
                handle_plain_ask()(first_message, "first request", target_thread_id=None)
            )
            self.assertTrue(await asyncio.to_thread(started.wait, 1))

            await handle_plain_ask()(second_message, "second request", target_thread_id=None)

            busy_messages = [
                (content, view)
                for content, view in channel.messages
                if isinstance(view, bot.BusyChoiceView)
            ]
            self.assertEqual(len(busy_messages), 1)
            _content, view = busy_messages[0]
            self.assertEqual(view.target_thread_id, "thread-1")
            async with bot.THREAD_RUNNERS_LOCK:
                self.assertNotIn(target_key, bot.THREAD_RUNNERS)
                self.assertNotIn(selected_key, bot.THREAD_RUNNERS)

            release.set()
            await asyncio.wait_for(first_task, timeout=1)
        finally:
            release.set()
            if first_task is not None and not first_task.done():
                _ = first_task.cancel()
                try:
                    await first_task
                except asyncio.CancelledError as exc:
                    _ = exc
            async with bot.THREAD_RUNNERS_LOCK:
                _ = bot.THREAD_RUNNERS.pop(target_key, None)
                _ = bot.THREAD_RUNNERS.pop(selected_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(target_key, None)
            _ = bot.get_runtime_state().ask_delivery_locks.pop(selected_key, None)
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

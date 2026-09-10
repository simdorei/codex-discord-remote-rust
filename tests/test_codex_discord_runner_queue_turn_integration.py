from __future__ import annotations

# pyright: reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
import asyncio  # noqa: ANYIO_OK
from contextlib import nullcontext
import os
from pathlib import Path
import tempfile
import threading
from typing import cast
import unittest
from unittest import mock

import codex_app_server_transport as app_server_transport
from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_app_server_transport_turn_outcomes import (
    TurnCompletion,
    TurnCompletionFound,
    TurnStatus,
)
import codex_discord_bot as bot
import codex_discord_prompt_delivery_prepare as prompt_delivery_prepare
import codex_discord_prompt_mapped_delivery as prompt_mapped_delivery
from codex_discord_runner_queue import QueueJob, QueueJobValue
import codex_discord_runtime as discord_runtime
import codex_discord_store as store

from tests.test_codex_discord_bot import FakeTarget


async def cleanup_runner(target_thread_id: str) -> None:
    runner = await bot.get_thread_runner(target_thread_id)
    task = runner["task"]
    if task is not None and not task.done():
        _ = task.cancel()
        try:
            await task
        except asyncio.CancelledError as exc:
            _ = exc
    async with bot.THREAD_RUNNERS_LOCK:
        _ = bot.THREAD_RUNNERS.pop(discord_runtime.normalize_runner_key(target_thread_id), None)


class DiscordRunnerQueueTurnIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_durable_queue_ignores_stale_mirror_busy_after_native_turn_is_idle(self) -> None:
        original_run_prompt_and_send = bot.run_prompt_and_send
        original_get_busy_state = bot.get_busy_state_for_thread
        original_mirror_db_path = bot.MIRROR_DB_PATH
        durable_deps = bot.RUNNER_RUNTIME.deps.durable_queue.deps
        original_lifecycle_getter = durable_deps.get_app_server_lifecycle
        original_admission = durable_deps.admit_app_server_generation
        calls: list[tuple[QueueJobValue, str, str | None, bool]] = []
        target_thread_id = "duck-channel-thread"

        async def fake_run_prompt_and_send(
            channel: QueueJobValue,
            prompt: str,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
            target_thread_id: str | None = None,
            expected_app_server_generation: int | None = None,
        ) -> prompt_delivery_prepare.PromptDeliveryPreparationResult:
            _ = (queued, source_message, expected_app_server_generation)
            calls.append((channel, prompt, target_thread_id, ack_sent))
            return prompt_delivery_prepare.PromptDeliveryPreparationResult(
                handled=True,
                target_thread_id=target_thread_id,
                target_ref=target_thread_id or "",
                recent_offsets={},
                delegate_to_session_mirror=False,
                mapped_result=prompt_mapped_delivery.MappedPromptDeliveryResult(
                    handled=True,
                    accepted=True,
                    turn_id="turn-1",
                ),
            )

        def stale_mirror_busy_state(_target: str | None) -> tuple[str, str | None, str]:
            return "busy", target_thread_id, target_thread_id

        try:
            object.__setattr__(
                durable_deps,
                "get_app_server_lifecycle",
                lambda: AppServerLifecycleSnapshot(1, True, 1.0),
            )
            object.__setattr__(
                durable_deps,
                "admit_app_server_generation",
                lambda _expected: nullcontext(
                    AppServerLifecycleSnapshot(1, True, 1.0)
                ),
            )
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with mock.patch.dict(
                    os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}
                ):
                    bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                    bot.run_prompt_and_send = fake_run_prompt_and_send
                    bot.get_busy_state_for_thread = stale_mirror_busy_state
                    completion = TurnCompletion(
                        thread_id=target_thread_id,
                        turn_id="turn-1",
                        status=TurnStatus.COMPLETED,
                    )
                    with (
                        mock.patch.object(
                            app_server_transport.PersistentCodexAppServer,
                            "get_thread_turn_states",
                            return_value={},
                        ),
                        mock.patch.object(
                            app_server_transport.PersistentCodexAppServer,
                            "wait_for_turn_completion",
                            return_value=TurnCompletionFound(completion),
                        ),
                    ):
                        channel = FakeTarget()
                        _ = await bot.enqueue_thread_ask(
                            channel,
                            "hello",
                            target_thread_id,
                            queued=True,
                            ack_sent=True,
                        )
                        runner = await bot.get_thread_runner(target_thread_id)
                        queue = cast(asyncio.Queue[QueueJob], runner["queue"])
                        _ = await asyncio.wait_for(queue.join(), timeout=1)

            self.assertEqual(calls, [(channel, "hello", target_thread_id, True)])
        finally:
            bot.MIRROR_DB_PATH = original_mirror_db_path
            bot.run_prompt_and_send = original_run_prompt_and_send
            bot.get_busy_state_for_thread = original_get_busy_state
            object.__setattr__(
                durable_deps,
                "get_app_server_lifecycle",
                original_lifecycle_getter,
            )
            object.__setattr__(
                durable_deps,
                "admit_app_server_generation",
                original_admission,
            )
            await cleanup_runner(target_thread_id)

    async def test_second_terminal_failure_flushes_pending_batch_without_running_next_job(self) -> None:
        original_run_prompt_and_send = bot.run_prompt_and_send
        original_mirror_db_path = bot.MIRROR_DB_PATH
        durable_deps = bot.RUNNER_RUNTIME.deps.durable_queue.deps
        original_lifecycle_getter = durable_deps.get_app_server_lifecycle
        original_admission = durable_deps.admit_app_server_generation
        calls: list[str] = []
        wait_gate = threading.Event()
        completion_index = 0

        async def fake_run_prompt_and_send(
            channel: QueueJobValue,
            prompt: str,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: QueueJobValue = None,
            target_thread_id: str | None = None,
            expected_app_server_generation: int | None = None,
        ) -> prompt_delivery_prepare.PromptDeliveryPreparationResult:
            _ = channel, queued, ack_sent, source_message, expected_app_server_generation
            calls.append(prompt)
            turn_id = f"turn-{len(calls)}"
            return prompt_delivery_prepare.PromptDeliveryPreparationResult(
                handled=True,
                target_thread_id=target_thread_id,
                target_ref=target_thread_id or "",
                recent_offsets={},
                delegate_to_session_mirror=False,
                mapped_result=prompt_mapped_delivery.MappedPromptDeliveryResult(
                    handled=True,
                    accepted=True,
                    turn_id=turn_id,
                ),
            )

        def wait_for_completion(
            thread_id: str,
            turn_id: str,
            *,
            timeout_sec: float,
            expected_generation: int | None = None,
        ) -> TurnCompletionFound:
            nonlocal completion_index
            _ = timeout_sec, expected_generation
            if not wait_gate.wait(timeout=2.0):
                raise TimeoutError("test wait gate did not open")
            completion_index += 1
            return TurnCompletionFound(
                TurnCompletion(
                    thread_id=thread_id,
                    turn_id=turn_id,
                    status=TurnStatus.FAILED,
                    error_message=f"failure {completion_index}",
                )
            )

        target_thread_id = "thread-failure-flush"
        try:
            object.__setattr__(
                durable_deps,
                "get_app_server_lifecycle",
                lambda: AppServerLifecycleSnapshot(1, True, 1.0),
            )
            object.__setattr__(
                durable_deps,
                "admit_app_server_generation",
                lambda _expected: nullcontext(
                    AppServerLifecycleSnapshot(1, True, 1.0)
                ),
            )
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with mock.patch.dict(
                    os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}
                ):
                    bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                    bot.run_prompt_and_send = fake_run_prompt_and_send
                    with (
                        mock.patch.object(
                            app_server_transport.PersistentCodexAppServer,
                            "get_thread_turn_states",
                            return_value={},
                        ),
                        mock.patch.object(
                            app_server_transport.PersistentCodexAppServer,
                            "wait_for_turn_completion",
                            side_effect=wait_for_completion,
                        ),
                    ):
                        channel = FakeTarget(channel_id=222)
                        _ = await bot.enqueue_thread_ask(
                            channel,
                            "first request",
                            target_thread_id,
                            queued=True,
                            ack_sent=True,
                        )
                        _ = await bot.enqueue_thread_ask(
                            channel,
                            "second request",
                            target_thread_id,
                            queued=True,
                            ack_sent=True,
                        )
                        wait_gate.set()
                        runner = await bot.get_thread_runner(target_thread_id)
                        queue = cast(asyncio.Queue[QueueJob], runner["queue"])
                        _ = await asyncio.wait_for(queue.join(), timeout=3)

                self.assertEqual(len(calls), 2)
                self.assertEqual(calls[0], "first request")
                self.assertIn("first request", calls[1])
                self.assertNotIn("second request", "\n".join(calls))
                self.assertEqual(store.list_queue_jobs(bot.MIRROR_DB_PATH), [])
                messages = "\n".join(message for message, _view in channel.messages)
                self.assertIn("Retrying this request once", messages)
                self.assertIn("failure 2", messages)
                self.assertIn("Deleted queued requests: 2", messages)
                self.assertIn("second request", messages)
        finally:
            wait_gate.set()
            bot.MIRROR_DB_PATH = original_mirror_db_path
            bot.run_prompt_and_send = original_run_prompt_and_send
            object.__setattr__(
                durable_deps,
                "get_app_server_lifecycle",
                original_lifecycle_getter,
            )
            object.__setattr__(
                durable_deps,
                "admit_app_server_generation",
                original_admission,
            )
            await cleanup_runner(target_thread_id)


if __name__ == "__main__":
    _ = unittest.main()

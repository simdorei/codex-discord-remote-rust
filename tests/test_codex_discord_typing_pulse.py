from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import unittest
from contextlib import suppress

import codex_discord_channel_typing as channel_typing
from codex_discord_typing_pulse import TypingPulseRegistry
from tests.test_codex_discord_channel_typing import RecordingTypingManager, TypingManagerError, TypingTarget


class TypingPulseRegistryTests(unittest.IsolatedAsyncioTestCase):
    async def test_stops_pulse_when_typing_start_fails(self) -> None:
        logs: list[str] = []
        stopped_targets: list[str] = []
        stopped = asyncio.Event()
        registry = TypingPulseRegistry(pulse_seconds=10.0, interval_seconds=0.1)

        async def on_start_error(target_thread_id: str) -> bool:
            stopped_targets.append(target_thread_id)
            registry.stop(target_thread_id)
            stopped.set()
            return True

        registry.start(
            TypingTarget(RecordingTypingManager(fail_enter=True)),
            "thread-1",
            "unit",
            channel_typing=lambda channel, *, context, raise_start_error=False: channel_typing.channel_typing(
                channel,
                context=context,
                log_func=logs.append,
                raise_start_error=raise_start_error,
            ),
            log=logs.append,
            on_start_error=on_start_error,
        )
        task = registry._tasks["thread-1"]

        await asyncio.wait_for(stopped.wait(), timeout=1.0)
        await asyncio.wait_for(task, timeout=1.0)
        await asyncio.sleep(0)

        self.assertEqual(stopped_targets, ["thread-1"])
        self.assertNotIn("thread-1", registry._tasks)
        self.assertEqual(
            logs,
            [
                "typing_pulse_started target=thread-1 context=unit",
                "typing_start_failed context=unit error_type=TypingManagerError",
                "typing_pulse_failed target=thread-1 context=unit error_type=TypingManagerError",
            ],
        )

    async def test_stop_from_worker_thread_does_not_need_running_event_loop(self) -> None:
        # Given: a registered pulse task owned by the async test loop.
        stopped = asyncio.Event()
        registry = TypingPulseRegistry()

        async def wait_forever() -> None:
            try:
                await asyncio.sleep(60)
            finally:
                stopped.set()

        task = asyncio.create_task(wait_forever())
        registry._tasks["thread-1"] = task

        try:
            # When: idle mirror cleanup stops the pulse from a worker thread.
            await asyncio.to_thread(registry.stop, "thread-1")

            # Then: stop cancels the task without requiring an event loop in that worker.
            await asyncio.wait_for(stopped.wait(), timeout=1.0)
            self.assertNotIn("thread-1", registry._tasks)
        finally:
            if not task.done():
                task.cancel()
            with suppress(asyncio.CancelledError):
                await task

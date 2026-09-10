from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from dataclasses import dataclass
from types import TracebackType
import unittest

import codex_discord_runtime as runtime


@dataclass(slots=True)  # noqa: MUTABLE_OK
class FakeAsyncLock:
    entered: bool = False

    async def __aenter__(self) -> None:
        self.entered = True

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc_value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc_value, traceback)


class RuntimeStateTests(unittest.IsolatedAsyncioTestCase):
    def test_runtime_helpers_preserve_keys_handoffs_and_relay_generation(self) -> None:
        # Given
        handoffs: dict[str, float] = {}
        generations: dict[str, int] = {}

        # When
        selected_handoff = runtime.mark_steering_handoff(
            handoffs,
            None,
            now_func=lambda: 10.0,
        )
        target_handoff = runtime.mark_steering_handoff(
            handoffs,
            "thread-1",
            now_func=lambda: 12.0,
        )
        first_generation = runtime.register_discord_relay(generations, "thread-1")
        second_generation = runtime.register_discord_relay(generations, "thread-1")

        # Then
        self.assertEqual(selected_handoff, 10.0)
        self.assertEqual(target_handoff, 12.0)
        self.assertEqual(handoffs, {"__selected__": 10.0, "thread-1": 12.0})
        self.assertTrue(runtime.had_steering_handoff_since(handoffs, None, 9.0))
        self.assertFalse(runtime.had_steering_handoff_since(handoffs, None, 10.0))
        self.assertEqual(first_generation, 1)
        self.assertEqual(second_generation, 2)
        self.assertTrue(runtime.is_discord_relay_stale(generations, "thread-1", 1))
        self.assertFalse(runtime.is_discord_relay_stale(generations, "thread-1", 2))

    async def test_build_runners_message_formats_empty_and_queued_runners(self) -> None:
        # Given
        lock = FakeAsyncLock()
        queue: asyncio.Queue[str] = asyncio.Queue()
        queue.put_nowait("pending")
        runners: dict[str, runtime.RunnerState] = {
            "abcdef123456": {
                "queue": queue,
                "active": True,
                "target_thread_id": "thread-1",
            },
        }

        def resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
            return target_thread_id, "taxlab:1"

        # When
        empty_message = await runtime.build_runners_message(
            {},
            lock,
            resolve_target_ref_func=resolve_target_ref,
        )
        queued_message = await runtime.build_runners_message(
            runners,
            lock,
            resolve_target_ref_func=resolve_target_ref,
        )

        # Then
        self.assertTrue(lock.entered)
        self.assertEqual(empty_message, "No active Discord runner queues.")
        self.assertEqual(
            queued_message,
            "Discord runner queues\n- taxlab:1: active=True queued=1 key=abcdef12",
        )

    async def test_build_runners_message_handles_partial_runner_record(self) -> None:
        lock = FakeAsyncLock()
        runners: dict[str, runtime.RunnerState] = {"xyz987654321": {}}

        def resolve_target_ref(target_thread_id: str | None) -> tuple[str | None, str]:
            return target_thread_id, "selected"

        message = await runtime.build_runners_message(
            runners,
            lock,
            resolve_target_ref_func=resolve_target_ref,
        )

        self.assertEqual(
            message,
            "Discord runner queues\n- selected: active=False queued=0 key=xyz98765",
        )


if __name__ == "__main__":
    _ = unittest.main()

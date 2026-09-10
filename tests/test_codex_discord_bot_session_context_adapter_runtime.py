from __future__ import annotations

from types import ModuleType
import unittest
from unittest import mock

import codex_discord_bot_session_context_adapter_runtime as adapter_runtime
from codex_app_server_transport_turn_outcomes import (
    InterruptOrigin,
    TurnCompletion,
    TurnStatus,
)


class BotSessionContextAdapterRuntimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.runtime = adapter_runtime.BotSessionContextAdapterRuntime(ModuleType("test_bot"))

    def test_complete_native_cache_skips_thread_read_and_keeps_newer_turns(self) -> None:
        completions = {
            "turn-completed": TurnCompletion(
                "thread-1",
                "turn-completed",
                TurnStatus.COMPLETED,
            ),
            "turn-failed": TurnCompletion(
                "thread-1",
                "turn-failed",
                TurnStatus.FAILED,
                error_message="worker crashed",
            ),
            "turn-interrupted": TurnCompletion(
                "thread-1",
                "turn-interrupted",
                TurnStatus.INTERRUPTED,
                interrupt_origin=InterruptOrigin.REMOTE_USER_INTENT,
            ),
            "turn-newer": TurnCompletion(
                "thread-1",
                "turn-newer",
                TurnStatus.COMPLETED,
            ),
        }

        with (
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                create=True,
                return_value=completions,
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "is_running",
                return_value=True,
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_thread_turn_completions",
            ) as read_completions,
        ):
            actual = self.runtime.get_thread_turn_completions(
                "thread-1",
                ["turn-completed", "turn-failed", "turn-interrupted", "turn-completed"],
            )

        self.assertEqual(actual, completions)
        self.assertEqual(
            actual["turn-interrupted"].interrupt_origin,
            InterruptOrigin.REMOTE_USER_INTENT,
        )
        read_completions.assert_not_called()

    def test_timeout_uses_cache_that_became_complete_during_thread_read(self) -> None:
        completed = TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED)
        failed = TurnCompletion(
            "thread-1",
            "turn-2",
            TurnStatus.FAILED,
            error_message="late native failure",
        )
        with (
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                create=True,
                side_effect=[
                    {"turn-1": completed},
                    {"turn-1": completed, "turn-2": failed},
                ],
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "is_running",
                return_value=True,
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_thread_turn_completions",
                side_effect=TimeoutError("thread/read timed out"),
            ) as read_completions,
        ):
            actual = self.runtime.get_thread_turn_completions(
                "thread-1",
                ["turn-1", "turn-2"],
            )

        self.assertEqual(actual, {"turn-1": completed, "turn-2": failed})
        read_completions.assert_called_once_with("thread-1", timeout_sec=3.0)

    def test_successful_thread_read_cannot_overwrite_cached_interruption(self) -> None:
        interrupted = TurnCompletion(
            "thread-1",
            "turn-1",
            TurnStatus.INTERRUPTED,
            interrupt_origin=InterruptOrigin.REMOTE_USER_INTENT,
        )
        read_completed = TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED)
        second_completed = TurnCompletion("thread-1", "turn-2", TurnStatus.COMPLETED)

        with (
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                create=True,
                side_effect=[{"turn-1": interrupted}, {"turn-1": interrupted}],
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "is_running",
                return_value=True,
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_thread_turn_completions",
                return_value={"turn-1": read_completed, "turn-2": second_completed},
            ) as read_completions,
        ):
            actual = self.runtime.get_thread_turn_completions(
                "thread-1",
                ["turn-1", "turn-2"],
            )

        self.assertEqual(actual, {"turn-1": interrupted, "turn-2": second_completed})
        read_completions.assert_called_once_with("thread-1", timeout_sec=3.0)

    def test_timeout_remains_fail_closed_when_cache_is_still_partial(self) -> None:
        completed = TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED)

        with (
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                create=True,
                side_effect=[{"turn-1": completed}, {"turn-1": completed}],
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "is_running",
                return_value=True,
            ),
            mock.patch.object(
                adapter_runtime.app_server_transport.DEFAULT_CLIENT,
                "get_thread_turn_completions",
                side_effect=TimeoutError("thread/read timed out"),
            ) as read_completions,
        ):
            with self.assertRaisesRegex(TimeoutError, "thread/read timed out"):
                _ = self.runtime.get_thread_turn_completions(
                    "thread-1",
                    ["turn-1", "turn-2"],
                )

        read_completions.assert_called_once_with("thread-1", timeout_sec=3.0)


if __name__ == "__main__":
    _ = unittest.main()

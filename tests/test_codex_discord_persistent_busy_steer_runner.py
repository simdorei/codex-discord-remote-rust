from __future__ import annotations

import unittest
from types import TracebackType

import codex_discord_persistent_busy_steer as persistent_busy_steer
from codex_discord_steering import SteeringPromptResult


class RecordingTypingContext:
    _events: list[str]
    _label: str

    def __init__(self, events: list[str], label: str) -> None:
        self._events = events
        self._label = label

    async def __aenter__(self) -> None:
        self._events.append(f"enter:{self._label}")

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc, tb)
        self._events.append(f"exit:{self._label}")


class PersistentBusySteerRunnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_runner_logs_elapsed_and_marks_success_handoff(self) -> None:
        calls: list[tuple[str, str | None]] = []
        marks: list[str | None] = []
        logs: list[str] = []
        typing_events: list[str] = []
        times = iter([10.0, 11.25])

        def run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            calls.append((prompt, target_thread_id))
            return SteeringPromptResult(0, "done", target_thread_id=target_thread_id)

        def channel_typing(channel: object, *, context: str) -> RecordingTypingContext:
            _ = channel
            return RecordingTypingContext(typing_events, context)

        result = await persistent_busy_steer.run_persistent_busy_steer_prompt(
            object(),
            "please steer",
            "thread-1",
            choice_id="0123456789abcdef01234567",
            deps=persistent_busy_steer.PersistentBusySteerRunDeps(
                run_steering_prompt=run_steering_prompt,
                channel_typing=channel_typing,
                mark_steering_handoff=marks.append,
                format_log_text_len=lambda text: len(text or ""),
                monotonic=lambda: next(times),
                log=logs.append,
            ),
        )

        self.assertEqual(result.exit_code, 0)
        self.assertEqual(calls, [("please steer", "thread-1")])
        self.assertEqual(marks, ["thread-1"])
        self.assertEqual(typing_events, ["enter:persistent_steer_now", "exit:persistent_steer_now"])
        self.assertEqual(
            logs,
            [
                "busy_choice_persistent_steer_done exit=0 choice=0123456789abcdef01234567 target=thread-1 elapsed_sec=1.25 output_len=4"
            ],
        )

    async def test_runner_does_not_mark_failure_and_logs_missing_target(self) -> None:
        marks: list[str | None] = []
        logs: list[str] = []
        typing_events: list[str] = []
        times = iter([1.0, 1.5])

        def run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            _ = (prompt, target_thread_id)
            return SteeringPromptResult(7, "oops")

        def channel_typing(channel: object, *, context: str) -> RecordingTypingContext:
            _ = channel
            return RecordingTypingContext(typing_events, context)

        result = await persistent_busy_steer.run_persistent_busy_steer_prompt(
            object(),
            "please steer",
            None,
            choice_id="choice-1",
            deps=persistent_busy_steer.PersistentBusySteerRunDeps(
                run_steering_prompt=run_steering_prompt,
                channel_typing=channel_typing,
                mark_steering_handoff=marks.append,
                format_log_text_len=lambda text: len(text or ""),
                monotonic=lambda: next(times),
                log=logs.append,
            ),
        )

        self.assertEqual(result.exit_code, 7)
        self.assertEqual(marks, [])
        self.assertEqual(typing_events, ["enter:persistent_steer_now", "exit:persistent_steer_now"])
        self.assertEqual(
            logs,
            ["busy_choice_persistent_steer_done exit=7 choice=choice-1 target=- elapsed_sec=0.50 output_len=4"],
        )

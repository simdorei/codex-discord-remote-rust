# pyright: reportAny=false, reportAttributeAccessIssue=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

import unittest
from pathlib import Path

import codex_desktop_bridge as bridge
import codex_desktop_bridge_final_answer as final_answer
import codex_desktop_bridge_final_answer_types as final_answer_types
from codex_app_server_transport_goal import GoalAbsent, GoalTransportError, ThreadGoalLookup, ThreadGoalStatus
from codex_app_server_transport_turn_outcomes import (
    InterruptOrigin,
    TurnCompletion,
    TurnCompletionFound,
    TurnCompletionObservation,
    TurnCompletionPending,
    TurnStatus,
)
from codex_session_events import JsonEvent, JsonValue


class FinalAnswerWatchHappyTests(unittest.TestCase):
    def test_watch_returns_final_with_commentary_stream_and_bridge_wrappers(self) -> None:
        events = [
            _agent_message("checking"),
            _agent_message("checking"),
            _assistant_message("commentary", "thinking"),
            _agent_final_message("done"),
            _assistant_message("final_answer", "done"),
            _task_complete("done"),
        ]
        callback_lines: list[str] = []
        fake_deps = _deps([events])

        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            stream_live=True,
            stream_label="thread-1",
            stream_callback=callback_lines.append,
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(result["status"], "final")
        self.assertEqual(result["commentary"], ["checking", "thinking"])
        self.assertEqual(result["final_answer"], "done")
        self.assertTrue(result["streamed_live"])
        self.assertTrue(result["final_streamed_live"])
        self.assertIn("thread-1 [commentary]", callback_lines)
        self.assertEqual(callback_lines.count("thread-1 [final_answer]"), 1)
        self.assertEqual(fake_deps.cursors, [0])

        original_read = bridge.read_new_session_events
        original_notice = bridge.build_interactive_notice_from_function_call
        original_extract = bridge.extract_message_text
        try:
            bridge.read_new_session_events = lambda session_path, cursor: (
                [_assistant_message("final_answer", "wrapped"), _task_complete("wrapped")],
                1,
            )
            bridge.build_interactive_notice_from_function_call = lambda payload: "[approval_required]"
            bridge.extract_message_text = lambda payload: "wrapped"
            wrapped = bridge.watch_for_final_answer(Path("session.jsonl"), 0, 1.0, True)
        finally:
            bridge.read_new_session_events = original_read
            bridge.build_interactive_notice_from_function_call = original_notice
            bridge.extract_message_text = original_extract
        self.assertEqual(wrapped["status"], "final")
        self.assertEqual(wrapped["final_answer"], "wrapped")


class FinalAnswerWatchEdgeTests(unittest.TestCase):
    def test_watch_handles_edge_payloads_abort_and_timeout(self) -> None:
        callback_lines: list[str] = []
        fake_deps = _deps(
            [
                [
                    {"type": "event_msg", "payload": []},
                    _agent_message("checking"),
                    _function_call("call-1"),
                    _function_output("Rejected by user: no"),
                    _assistant_message("commentary", ""),
                    _assistant_message("commentary", "checking"),
                    _event_msg("turn_aborted"),
                ],
            ],
        )

        aborted = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            stream_live=True,
            stream_label="thread-2",
            stream_callback=callback_lines.append,
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(aborted["status"], "aborted")
        self.assertEqual(
            aborted["commentary"],
            ["checking", "[approval_required]\ncall-1", "[approval_rejected]\nCommand approval was rejected by user.", "checking"],
        )
        self.assertTrue(aborted["streamed_live"])
        self.assertFalse(aborted["final_streamed_live"])
        self.assertIn("thread-2 [commentary]", callback_lines)

        timeout = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=5,
            timeout_sec=1.0,
            include_commentary=True,
            deps=_deps([[]], times=[0.0, 2.0]).as_deps(),
        )

        self.assertEqual(timeout["status"], "timeout")
        self.assertEqual(timeout["commentary"], [])
        self.assertEqual(timeout["final_answer"], "")

    def test_final_answer_candidate_does_not_hide_later_turn_abort(self) -> None:
        fake_deps = _deps(
            [
                [
                    _assistant_message("final_answer", "not actually complete"),
                    _event_msg("turn_aborted"),
                ]
            ]
        )

        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(result["status"], "aborted")
        self.assertEqual(result["final_answer"], "")

    def test_active_goal_turn_completion_is_progress_not_final(self) -> None:
        callback_lines: list[str] = []
        fake_deps = _deps(
            [[_assistant_message("final_answer", "still working"), _task_complete("still working")]],
            goal_status=ThreadGoalStatus.ACTIVE,
        )

        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            stream_live=True,
            stream_callback=callback_lines.append,
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(result["status"], "progress")
        self.assertEqual(result["commentary"], ["still working"])
        self.assertEqual(result["final_answer"], "")
        self.assertIn("[commentary]", callback_lines)
        self.assertNotIn("[final_answer]", callback_lines)

    def test_complete_goal_turn_completion_is_final(self) -> None:
        callback_lines: list[str] = []
        fake_deps = _deps(
            [[_assistant_message("final_answer", "goal complete"), _task_complete("goal complete")]],
            goal_status=ThreadGoalStatus.COMPLETE,
        )

        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            stream_live=True,
            stream_callback=callback_lines.append,
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(result["status"], "final")
        self.assertEqual(result["final_answer"], "goal complete")
        self.assertEqual(callback_lines.count("[final_answer]"), 1)

    def test_native_interrupted_turn_ends_without_rollout_terminal(self) -> None:
        completion = TurnCompletion(
            thread_id="thread-1",
            turn_id="turn-1",
            status=TurnStatus.INTERRUPTED,
            interrupt_origin=InterruptOrigin.REMOTE_USER_INTENT,
        )
        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            expected_turn_id="turn-1",
            deps=_deps([[]], observations=[TurnCompletionFound(completion)]).as_deps(),
        )

        self.assertEqual(result["status"], "aborted")
        self.assertEqual(result.get("interrupt_origin"), "remote_user_intent")
        self.assertEqual(result["final_answer"], "")

    def test_native_failed_turn_preserves_error_message(self) -> None:
        completion = TurnCompletion(
            thread_id="thread-1",
            turn_id="turn-1",
            status=TurnStatus.FAILED,
            error_message="model process exited",
        )
        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            expected_turn_id="turn-1",
            deps=_deps([[]], observations=[TurnCompletionFound(completion)]).as_deps(),
        )

        self.assertEqual(result["status"], "failed")
        self.assertEqual(result.get("error_message"), "model process exited")
        self.assertEqual(result["final_answer"], "")

    def test_native_watch_ignores_wrong_turn_rollout_terminal(self) -> None:
        completed = TurnCompletion("thread-1", "turn-2", TurnStatus.COMPLETED)
        fake_deps = _deps(
            [[_task_complete_for_turn("turn-1", "old reply")], [_task_complete_for_turn("turn-2", "new reply")]],
            times=[0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 0.5],
            observations=[TurnCompletionPending(), TurnCompletionPending(), TurnCompletionFound(completed)],
        )

        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            expected_turn_id="turn-2",
            deps=fake_deps.as_deps(),
        )

        self.assertEqual(result["status"], "final")
        self.assertEqual(result["final_answer"], "new reply")

    def test_native_completion_without_exact_rollout_ignores_phase_candidate(self) -> None:
        completed = TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED)
        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            expected_turn_id="turn-1",
            deps=_deps(
                [[_agent_final_message("stale candidate")], []],
                times=[0.0, 0.0, 0.0, 0.0, 2.0, 2.0],
                observations=[TurnCompletionPending(), TurnCompletionFound(completed), TurnCompletionFound(completed)],
            ).as_deps(),
        )

        self.assertEqual(result["status"], "final")
        self.assertEqual(result["final_answer"], "Codex turn completed without a visible reply.")

    def test_native_goal_lookup_error_is_transport_error_not_final(self) -> None:
        completed = TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED)
        result = final_answer.watch_for_final_answer(
            Path("session.jsonl"),
            start_offset=0,
            timeout_sec=5.0,
            include_commentary=True,
            expected_turn_id="turn-1",
            deps=_deps(
                [[_task_complete("done")]],
                observations=[TurnCompletionPending(), TurnCompletionFound(completed)],
                goal_lookup=GoalTransportError("goal lookup failed"),
            ).as_deps(),
        )

        self.assertEqual(result["status"], "transport_error")
        self.assertEqual(result.get("error_message"), "goal lookup failed")
        self.assertEqual(result["final_answer"], "")


class FinalAnswerWatchPublicSurfaceTests(unittest.TestCase):
    def test_emit_watch_stream_block_sends_marker_text_and_trailing_blank(self) -> None:
        lines: list[str] = []

        final_answer.emit_watch_stream_block("[commentary]", "first\nsecond", stream_callback=lines.append)

        self.assertEqual(lines, ["[commentary]", "first", "second", ""])

    def test_watch_state_accumulates_mutable_result_fields(self) -> None:
        state = final_answer_types.WatchState()
        state.commentary.append("checking")
        state.seen_agent_messages.add("checking")
        state.seen_interactive_notices.add("call-1")
        state.final_answer = "done"
        state.did_stream_live = True
        state.did_stream_final_live = True

        result = final_answer_types.result_from_state("final", state)

        self.assertEqual(result["status"], "final")
        self.assertEqual(result["commentary"], ["checking"])
        self.assertEqual(result["final_answer"], "done")
        self.assertTrue(result["streamed_live"])
        self.assertTrue(result["final_streamed_live"])
        self.assertEqual(state.seen_agent_messages, {"checking"})
        self.assertEqual(state.seen_interactive_notices, {"call-1"})


class _FakeDeps:
    def __init__(
        self,
        event_batches: list[list[JsonEvent]],
        *,
        times: list[float] | None = None,
        goal_status: ThreadGoalStatus | None = None,
        observations: list[TurnCompletionObservation] | None = None,
        goal_lookup: ThreadGoalLookup | None = None,
    ) -> None:
        self.event_batches: list[list[JsonEvent]] = list(event_batches)
        self.times: list[float] = list(times or [0.0, 0.0, 0.0])
        self.cursors: list[int] = []
        self.goal_status: ThreadGoalStatus | None = goal_status
        self.observations: list[TurnCompletionObservation] = list(observations or [TurnCompletionPending()])
        self.goal_lookup: ThreadGoalLookup = GoalAbsent() if goal_lookup is None else goal_lookup

    def as_deps(self) -> final_answer.FinalAnswerWatchDeps:
        return final_answer.FinalAnswerWatchDeps(
            time_now=self._time_now,
            sleep=lambda seconds: None,
            read_new_session_events=self._read,
            build_interactive_notice_from_function_call=self._notice,
            extract_message_text=_extract_message_text,
            emit_watch_stream_block=final_answer.emit_watch_stream_block,
            get_thread_goal_status=lambda session_path: self.goal_status,
            observe_turn_completion=self._observe_turn_completion,
            get_thread_goal_lookup=lambda session_path: self.goal_lookup,
        )

    def _observe_turn_completion(self, session_path: Path, turn_id: str) -> TurnCompletionObservation:
        _ = session_path, turn_id
        if len(self.observations) == 1:
            return self.observations[0]
        return self.observations.pop(0)

    def _time_now(self) -> float:
        if len(self.times) == 1:
            return self.times[0]
        return self.times.pop(0)

    def _read(self, session_path: Path, cursor: int) -> tuple[list[JsonEvent], int]:
        _ = session_path
        self.cursors.append(cursor)
        if not self.event_batches:
            return [], cursor
        return self.event_batches.pop(0), cursor + 1

    def _notice(self, payload: final_answer.JsonObject) -> str:
        return f"[approval_required]\n{payload.get('call_id') or '-'}"


def _deps(
    event_batches: list[list[JsonEvent]],
    *,
    times: list[float] | None = None,
    goal_status: ThreadGoalStatus | None = None,
    observations: list[TurnCompletionObservation] | None = None,
    goal_lookup: ThreadGoalLookup | None = None,
) -> _FakeDeps:
    return _FakeDeps(
        event_batches,
        times=times,
        goal_status=goal_status,
        observations=observations,
        goal_lookup=goal_lookup,
    )


def _extract_message_text(payload: final_answer.JsonObject) -> str:
    content = payload.get("content")
    if not isinstance(content, list):
        return ""
    parts: list[str] = []
    for item in content:
        if isinstance(item, dict) and item.get("type") == "output_text":
            text = item.get("text")
            if isinstance(text, str):
                parts.append(text)
    return "\n".join(parts).strip()


def _event_msg(event_type: str) -> JsonEvent:
    return {"type": "event_msg", "payload": {"type": event_type}}


def _task_complete(last_agent_message: str | None) -> JsonEvent:
    return _task_complete_for_turn("turn-1", last_agent_message)


def _task_complete_for_turn(turn_id: str, last_agent_message: str | None) -> JsonEvent:
    return {
        "type": "event_msg",
        "payload": {
            "type": "task_complete",
            "turn_id": turn_id,
            "last_agent_message": last_agent_message,
        },
    }


def _agent_message(message: str) -> JsonEvent:
    return {"type": "event_msg", "payload": {"type": "agent_message", "message": message}}


def _agent_final_message(message: str) -> JsonEvent:
    return {
        "type": "event_msg",
        "payload": {
            "type": "agent_message",
            "phase": "final_answer",
            "message": message,
        },
    }


def _assistant_message(phase: str, text: str) -> JsonEvent:
    return {
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": "assistant",
            "phase": phase,
            "content": [{"type": "output_text", "text": text}],
        },
    }


def _function_call(call_id: str) -> JsonEvent:
    return {"type": "response_item", "payload": {"type": "function_call", "call_id": call_id}}


def _function_output(output: str) -> JsonEvent:
    payload: dict[str, JsonValue] = {"type": "function_call_output", "output": output}
    return {"type": "response_item", "payload": payload}


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

import unittest
from unittest import mock

import codex_discord_bot as bot
import codex_discord_session_mirror as session_mirror
from codex_app_server_transport_goal import GoalTransportError, ThreadGoalStatus, ThreadGoalUpdate
from codex_app_server_transport_turn_outcomes import (
    InterruptOrigin,
    TurnCompletion,
    TurnStatus,
)
from codex_session_events import JsonEvent


class SessionMirrorItemCollectionIntegrationTests(unittest.TestCase):
    def test_collect_session_mirror_items_skips_discord_echo_and_duplicate_commentary(self) -> None:
        old_prompts = dict(bot.get_runtime_state().recent_discord_origin_prompts)
        try:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.mark_recent_discord_origin_prompt("thread-1", "from discord")
            events: list[JsonEvent] = [
                {
                    "timestamp": "1",
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": "from discord"},
                },
                {
                    "timestamp": "2",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": "from app"}],
                    },
                },
                {
                    "timestamp": "3",
                    "type": "event_msg",
                    "payload": {"type": "agent_message", "phase": "commentary", "message": "working"},
                },
                {
                    "timestamp": "4",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "phase": "commentary",
                        "content": [{"type": "output_text", "text": "working"}],
                    },
                },
                {
                    "timestamp": "5",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "phase": "final_answer",
                        "content": [{"type": "output_text", "text": "done"}],
                    },
                },
                {
                    "timestamp": "6",
                    "type": "event_msg",
                    "payload": {
                        "type": "task_complete",
                        "turn_id": "turn-1",
                        "last_agent_message": "done",
                    },
                },
            ]

            with mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_thread_goal_status",
                return_value=None,
            ):
                items = bot.collect_session_mirror_items(
                    "thread-1",
                    events,
                    seen_agent_messages={},
                    seen_user_messages={},
                )

            self.assertEqual([item["kind"] for item in items], ["user", "commentary", "final"])
            self.assertEqual([item["text"] for item in items], ["from app", "working", "done"])
        finally:
            bot.get_runtime_state().recent_discord_origin_prompts.clear()
            bot.get_runtime_state().recent_discord_origin_prompts.update(old_prompts)

    def test_collect_session_mirror_items_skips_internal_user_context(self) -> None:
        user_text = "AGENTS.md \ub0b4\uc6a9 \uc124\uba85\ud574\uc918"
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "# AGENTS.md instructions for C:\\repos\\simdorei\\codex-discord-remote\n\n<INSTRUCTIONS>",
                        }
                    ],
                },
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": '<codex_internal_context source="goal">\nContinue working toward the active thread goal.',
                        }
                    ],
                },
            },
            {
                "timestamp": "3",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": user_text},
            },
            {
                "timestamp": "4",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "from app"}],
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["user", "user"])
        self.assertEqual([item["text"] for item in items], [user_text, "from app"])

    def test_collect_session_mirror_items_does_not_publish_phase_final_before_turn_outcome(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {"type": "agent_message", "phase": "final_answer", "message": "done"},
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{"type": "output_text", "text": "done"}],
                },
            },
            {
                "timestamp": "3",
                "type": "response_item",
                "payload": {
                    "type": "agent_message",
                    "author": "/root/worker",
                    "message": "subagent result",
                },
            },
        ]

        items = bot.collect_session_mirror_items(
            "thread-1",
            events,
            seen_agent_messages={},
            seen_user_messages={},
        )

        self.assertEqual(items, [])

    def test_collect_session_mirror_items_uses_task_complete_as_the_final_boundary(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{"type": "output_text", "text": "done"}],
                },
            },
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "done",
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["final"])
        self.assertEqual([item["text"] for item in items], ["done"])

    def test_collect_session_mirror_items_marks_active_goal_turn_complete_as_progress(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{"type": "output_text", "text": "more work remains"}],
                },
            },
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "more work remains",
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=ThreadGoalStatus.ACTIVE,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["commentary"])
        self.assertEqual(items[0]["phase"], "goal_turn_complete")
        self.assertEqual(session_mirror.format_session_mirror_text(items[0]), "In progress\n\nmore work remains")

    def test_collect_session_mirror_items_surfaces_aborted_event_details(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "turn_aborted",
                    "turn_id": "turn-1",
                    "reason": "interrupted",
                    "duration_ms": 123,
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["aborted"])
        self.assertEqual(items[0]["phase"], "turn_aborted")
        self.assertIn("Codex turn aborted.", items[0]["text"])
        self.assertIn("reason=interrupted", items[0]["text"])
        self.assertIn("turn_id=turn-1", items[0]["text"])
        self.assertIn("duration_ms=123", items[0]["text"])
        self.assertEqual(session_mirror.format_session_mirror_text(items[0]), items[0]["text"])

    def test_collect_session_mirror_items_surfaces_completed_turn_without_visible_reply(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "ping"},
            },
            {
                "timestamp": "2",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "ping"}],
                },
            },
            {
                "timestamp": "3",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": None,
                    "duration_ms": 2500,
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["user", "final"])
        self.assertEqual(items[-1]["phase"], "no_visible_reply")
        self.assertIn("Codex turn completed without a visible reply.", items[-1]["text"])
        self.assertIn("turn_id=turn-1", items[-1]["text"])
        self.assertIn("duration_ms=2500", items[-1]["text"])

    def test_abort_in_one_turn_does_not_suppress_replacement_turn_in_same_batch(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {"type": "turn_aborted", "turn_id": "turn-1", "reason": "interrupted"},
            },
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-2",
                    "last_agent_message": "replacement done",
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["aborted", "final"])
        self.assertEqual(items[-1]["text"], "replacement done")

    def test_complete_goal_snapshot_only_marks_latest_completed_turn_final(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "first progress",
                },
            },
            {
                "timestamp": "2",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-2",
                    "last_agent_message": "actual final",
                },
            },
        ]

        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=ThreadGoalStatus.COMPLETE,
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["commentary", "final"])
        self.assertEqual([item["text"] for item in items], ["first progress", "actual final"])

    def test_native_interrupted_status_vetoes_false_rollout_final(self) -> None:
        events = [_task_complete_event("turn-1", "must not be final")]
        interrupted = TurnCompletion(
            "thread-1",
            "turn-1",
            TurnStatus.INTERRUPTED,
            interrupt_origin=InterruptOrigin.EXTERNAL_OR_UNKNOWN,
        )
        with (
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                return_value={"turn-1": interrupted},
            ),
            mock.patch.object(bot.app_server_transport.DEFAULT_CLIENT, "get_thread_goal_status", return_value=None),
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["aborted"])
        self.assertIn("external_or_unknown", items[0]["text"])

    def test_native_failed_status_vetoes_false_rollout_final(self) -> None:
        events = [_task_complete_event("turn-1", "must not be final")]
        failed = TurnCompletion("thread-1", "turn-1", TurnStatus.FAILED, error_message="worker crashed")
        with (
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_cached_thread_turn_completions",
                return_value={"turn-1": failed},
            ),
            mock.patch.object(bot.app_server_transport.DEFAULT_CLIENT, "get_thread_goal_status", return_value=None),
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["failed"])
        self.assertEqual(items[0]["text"], "worker crashed")

    def test_goal_lookup_error_is_surfaced_instead_of_false_final(self) -> None:
        events = [_task_complete_event("turn-1", "unverified")]
        with mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_lookup",
            return_value=GoalTransportError("goal transport unavailable"),
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["transport_error"])
        self.assertEqual(items[0]["text"], "goal transport unavailable")

    def test_exact_goal_update_is_stronger_than_latest_goal_snapshot(self) -> None:
        events = [_task_complete_event("turn-1", "more work remains")]
        update = ThreadGoalUpdate("thread-1", "turn-1", ThreadGoalStatus.ACTIVE)
        with (
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_thread_goal_status",
                return_value=ThreadGoalStatus.COMPLETE,
            ),
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_cached_goal_update",
                return_value=update,
            ),
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["commentary"])
        self.assertEqual(items[0]["phase"], "goal_turn_complete")

    def test_native_thread_history_keeps_older_backlog_completion_as_progress(self) -> None:
        events = [_task_complete_event("turn-1", "older completion")]
        completions = {
            "turn-1": TurnCompletion("thread-1", "turn-1", TurnStatus.COMPLETED),
            "turn-2": TurnCompletion("thread-1", "turn-2", TurnStatus.COMPLETED),
        }
        with (
            mock.patch.object(bot.app_server_transport.DEFAULT_CLIENT, "is_running", return_value=True),
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_thread_turn_completions",
                return_value=completions,
            ),
            mock.patch.object(
                bot.app_server_transport.DEFAULT_CLIENT,
                "get_thread_goal_status",
                return_value=ThreadGoalStatus.COMPLETE,
            ),
        ):
            items = bot.collect_session_mirror_items(
                "thread-1",
                events,
                seen_agent_messages={},
                seen_user_messages={},
            )

        self.assertEqual([item["kind"] for item in items], ["commentary"])
        self.assertEqual(items[0]["phase"], "goal_turn_complete")


def _task_complete_event(turn_id: str, text: str) -> JsonEvent:
    return {
        "timestamp": "terminal",
        "type": "event_msg",
        "payload": {"type": "task_complete", "turn_id": turn_id, "last_agent_message": text},
    }


if __name__ == "__main__":
    _ = unittest.main()

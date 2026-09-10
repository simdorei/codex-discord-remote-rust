from __future__ import annotations

import unittest

from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject
from codex_app_server_transport_turn_outcomes import (
    InterruptOrigin,
    TurnStatus,
    parse_thread_turn_completions,
    parse_thread_turn_states,
    parse_turn_completion_notification,
)


class TurnOutcomeParserTests(unittest.TestCase):
    def test_notification_parses_all_terminal_statuses_from_nested_turn(self) -> None:
        completed = parse_turn_completion_notification(
            {"threadId": "thread-1", "turn": {"id": "turn-1", "status": "completed", "items": []}}
        )
        interrupted = parse_turn_completion_notification(
            {"threadId": "thread-1", "turn": {"id": "turn-2", "status": "interrupted", "items": []}},
            remote_user_intent=True,
        )
        failed = parse_turn_completion_notification(
            {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-3",
                    "status": "failed",
                    "items": [],
                    "error": {"message": "safe failure", "additionalDetails": "not surfaced"},
                },
            }
        )

        assert completed is not None and interrupted is not None and failed is not None
        self.assertIs(completed.status, TurnStatus.COMPLETED)
        self.assertIs(interrupted.status, TurnStatus.INTERRUPTED)
        self.assertIs(interrupted.interrupt_origin, InterruptOrigin.REMOTE_USER_INTENT)
        self.assertIs(failed.status, TurnStatus.FAILED)
        self.assertEqual(failed.error_message, "safe failure")

    def test_notification_rejects_nonterminal_completed_payload(self) -> None:
        with self.assertRaisesRegex(CodexAppServerTransportError, "inProgress"):
            _ = parse_turn_completion_notification(
                {"threadId": "thread-1", "turn": {"id": "turn-1", "status": "inProgress", "items": []}}
            )

    def test_thread_read_builds_exact_terminal_map_and_skips_active_turn(self) -> None:
        result: JsonObject = {
            "thread": {
                "id": "thread-1",
                "turns": [
                    {"id": "turn-1", "status": "completed", "items": []},
                    {"id": "turn-2", "status": "inProgress", "items": []},
                ],
            }
        }

        completions = parse_thread_turn_completions(result, expected_thread_id="thread-1")

        self.assertEqual(list(completions), ["turn-1"])
        self.assertIs(completions["turn-1"].status, TurnStatus.COMPLETED)

    def test_thread_read_state_map_keeps_active_turn_for_restart_reconciliation(self) -> None:
        result: JsonObject = {
            "thread": {
                "id": "thread-1",
                "turns": [
                    {"id": "turn-1", "status": "completed", "items": []},
                    {"id": "turn-2", "status": "inProgress", "items": []},
                ],
            }
        }

        states = parse_thread_turn_states(result, expected_thread_id="thread-1")

        self.assertEqual(list(states), ["turn-1", "turn-2"])
        self.assertIs(states["turn-2"].status, TurnStatus.IN_PROGRESS)


if __name__ == "__main__":
    _ = unittest.main()

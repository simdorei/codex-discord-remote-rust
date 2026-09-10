from __future__ import annotations

import unittest
from typing import override

from codex_app_server_transport_pending import PendingRequestState
from codex_app_server_transport_replies import JsonObject
from codex_app_server_transport_turn_outcomes import InterruptOrigin


class PendingRequestStateTests(unittest.TestCase):
    @override
    def __init__(self, methodName: str = "runTest") -> None:
        super().__init__(methodName)
        self.logs: list[str] = []

    @override
    def setUp(self) -> None:
        self.logs.clear()

    def test_pending_requests_keep_order_and_filter_by_thread(self) -> None:
        state = PendingRequestState()
        first: JsonObject = {
            "id": "approval-1",
            "method": "item/commandExecution/requestApproval",
            "params": {"threadId": "thread-1"},
        }
        second: JsonObject = {
            "id": "input-1",
            "method": "item/tool/requestUserInput",
            "params": {"threadId": "thread-2"},
        }

        state.record_server_request("approval-1", first, self.logs.append)
        state.record_server_request("input-1", second, self.logs.append)

        self.assertEqual([request["id"] for request in state.pending_requests()], ["approval-1", "input-1"])
        self.assertEqual([request["id"] for request in state.pending_requests("thread-2")], ["input-1"])

    def test_latest_requests_ignore_other_threads_and_methods(self) -> None:
        state = PendingRequestState()
        approval: JsonObject = {
            "id": "approval-1",
            "method": "item/fileChange/requestApproval",
            "params": {"threadId": "thread-1"},
        }
        input_request: JsonObject = {
            "id": "input-1",
            "method": "item/tool/requestUserInput",
            "params": {"threadId": "thread-1"},
        }

        state.record_server_request("approval-1", approval, self.logs.append)
        state.record_server_request("input-1", input_request, self.logs.append)

        self.assertIs(state.latest_approval_request("thread-1"), approval)
        self.assertIs(state.latest_input_request("thread-1"), input_request)
        self.assertIsNone(state.latest_approval_request("thread-2"))

    def test_url_mcp_elicitation_is_exposed_as_pending_approval(self) -> None:
        state = PendingRequestState()
        request: JsonObject = {
            "id": "elicitation-1",
            "method": "mcpServer/elicitation/request",
            "params": {
                "threadId": "thread-1",
                "mode": "url",
                "message": "Install Google Drive and continue.",
                "url": "codex://plugins/install/?marketplace=openai-curated",
            },
        }

        state.record_server_request("elicitation-1", request, self.logs.append)

        self.assertIs(
            state.latest_approval_request("thread-1"),
            request,
            "URL MCP elicitation must reach the Discord approval path.",
        )

    def test_notifications_track_active_turn_until_matching_completion(self) -> None:
        state = PendingRequestState()

        state.record_notification(
            {"method": "turn/started", "params": {"threadId": "thread-1", "turnId": "turn-1"}},
            self.logs.append,
        )
        state.record_notification(
            {"method": "turn/completed", "params": {"threadId": "thread-1", "turnId": "turn-other"}},
            self.logs.append,
        )

        self.assertEqual(state.active_turn_id("thread-1"), "turn-1")

        state.record_notification(
            {"method": "turn/completed", "params": {"threadId": "thread-1", "turnId": "turn-1"}},
            self.logs.append,
        )

        self.assertIsNone(state.active_turn_id("thread-1"))

    def test_completed_notification_preserves_failed_status_and_error_by_exact_turn(self) -> None:
        state = PendingRequestState()

        state.record_notification(
            {
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {
                        "id": "turn-1",
                        "status": "failed",
                        "items": [],
                        "error": {"message": "model request failed"},
                    },
                },
            },
            self.logs.append,
        )

        completion = state.turn_completion("thread-1", "turn-1")
        self.assertIsNotNone(completion)
        assert completion is not None
        self.assertEqual(completion.status.value, "failed")
        self.assertEqual(completion.error_message, "model request failed")

    def test_completed_notification_keeps_exact_interrupted_turn_without_active_start(self) -> None:
        state = PendingRequestState()

        state.record_notification(
            {
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {"id": "turn-2", "status": "interrupted", "items": []},
                },
            },
            self.logs.append,
        )

        completion = state.turn_completion("thread-1", "turn-2")
        self.assertIsNotNone(completion)
        assert completion is not None
        self.assertEqual(completion.status.value, "interrupted")
        self.assertIsNone(state.active_turn_id("thread-1"))

    def test_turn_completions_for_thread_excludes_other_threads_and_keeps_latest_status(self) -> None:
        state = PendingRequestState()
        for thread_id, status in (
            ("thread-1", "completed"),
            ("thread-2", "failed"),
            ("thread-1", "interrupted"),
        ):
            state.record_notification(
                {
                    "method": "turn/completed",
                    "params": {
                        "threadId": thread_id,
                        "turn": {
                            "id": "turn-1",
                            "status": status,
                            "items": [],
                            **({"error": {"message": "other thread"}} if status == "failed" else {}),
                        },
                    },
                },
                self.logs.append,
            )

        completions = state.turn_completions_for_thread("thread-1")

        self.assertEqual(list(completions), ["turn-1"])
        self.assertEqual(completions["turn-1"].status.value, "interrupted")

    def test_only_registered_exact_interrupt_is_attributed_to_remote_user_intent(self) -> None:
        state = PendingRequestState()
        state.record_notification(
            {
                "method": "turn/started",
                "params": {"threadId": "thread-1", "turn": {"id": "turn-1", "status": "inProgress"}},
            },
            self.logs.append,
        )
        self.assertTrue(state.register_remote_interrupt_intent("thread-1", "turn-1", registered_at=1.0))

        state.record_notification(
            {
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {"id": "turn-1", "status": "interrupted", "items": []},
                },
            },
            self.logs.append,
            now=2.0,
        )

        completion = state.turn_completion("thread-1", "turn-1")
        assert completion is not None
        self.assertIs(completion.interrupt_origin, InterruptOrigin.REMOTE_USER_INTENT)

    def test_generic_interruption_stays_external_or_unknown(self) -> None:
        state = PendingRequestState()

        state.record_notification(
            {
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turn": {"id": "turn-1", "status": "interrupted", "items": []},
                },
            },
            self.logs.append,
        )

        completion = state.turn_completion("thread-1", "turn-1")
        assert completion is not None
        self.assertIs(completion.interrupt_origin, InterruptOrigin.EXTERNAL_OR_UNKNOWN)

    def test_resolve_request_removes_it_from_pending_requests(self) -> None:
        state = PendingRequestState()
        request: JsonObject = {
            "id": "input-1",
            "method": "item/tool/requestUserInput",
            "params": {"threadId": "thread-1"},
        }

        state.record_server_request("input-1", request, self.logs.append)
        state.resolve_request("input-1")

        self.assertEqual(state.pending_requests("thread-1"), [])


if __name__ == "__main__":
    _ = unittest.main()

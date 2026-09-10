from __future__ import annotations

from collections.abc import Callable
from typing import override
import unittest

import codex_discord_app_server as app_server
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject


class FailingAppServerClient:
    def __init__(self, *, log_func: Callable[[str], None] | None = None) -> None:
        self.log_func: Callable[[str], None] | None = log_func

    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject:
        _ = include_turns
        return {"thread": {"id": thread_id}}

    def resume_thread(self, thread_id: str, *, timeout_sec: float = 10.0) -> JsonObject:
        _ = timeout_sec
        return {"thread": {"id": thread_id}}

    def start_turn(self, thread_id: str, prompt: str) -> JsonObject:
        _ = (thread_id, prompt)
        return {"turn": {"id": "turn-1"}}

    def steer_turn(self, thread_id: str, prompt: str, *, expected_turn_id: str) -> JsonObject:
        _ = (thread_id, prompt)
        return {"turn": {"id": expected_turn_id}}

    def get_active_turn_id(self, thread_id: str) -> str | None:
        _ = thread_id
        return None

    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None:
        _ = thread_id
        return {"request_id": "approval-1"}

    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None:
        _ = thread_id
        return {"request_id": "input-1"}

    def reply_to_pending_approval(self, thread_id: str, answer_text: str) -> JsonObject:
        _ = (thread_id, answer_text)
        raise CodexAppServerTransportError("approval write failed")

    def reply_to_pending_input(self, thread_id: str, answer_text: str) -> JsonObject:
        _ = (thread_id, answer_text)
        raise CodexAppServerTransportError("input write failed")


class ApprovalLookupFailingClient(FailingAppServerClient):
    @override
    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None:
        _ = thread_id
        raise CodexAppServerTransportError("approval lookup failed")


class InputLookupFailingClient(FailingAppServerClient):
    @override
    def get_latest_pending_approval_request(self, thread_id: str) -> JsonObject | None:
        _ = thread_id
        return None

    @override
    def get_latest_pending_input_request(self, thread_id: str) -> JsonObject | None:
        _ = thread_id
        raise CodexAppServerTransportError("input lookup failed")


class DiscordAppServerTests(unittest.TestCase):
    def test_submit_approval_reply_surfaces_transport_error(self) -> None:
        result = app_server.submit_approval_reply("thread-1", "1", client=FailingAppServerClient())

        self.assertEqual(
            result,
            (1, "ERROR: resident app-server approval writeback failed: approval write failed"),
        )

    def test_submit_input_reply_surfaces_transport_error(self) -> None:
        result = app_server.submit_input_reply("thread-1", "hello", client=FailingAppServerClient())

        self.assertEqual(
            result,
            (1, "ERROR: resident app-server input writeback failed: input write failed"),
        )

    def test_pending_state_logs_approval_failure_then_falls_back_to_input(self) -> None:
        logs: list[str] = []

        result = app_server.get_pending_interactive_state(
            "thread-1",
            client=ApprovalLookupFailingClient(log_func=logs.append),
        )

        self.assertEqual(result, "input")
        self.assertEqual(len(logs), 1)
        self.assertIn("kind=approval", logs[0])
        self.assertIn("error_type=CodexAppServerTransportError", logs[0])

    def test_pending_state_logs_input_failure_then_returns_none(self) -> None:
        logs: list[str] = []

        result = app_server.get_pending_interactive_state(
            "thread-1",
            client=InputLookupFailingClient(log_func=logs.append),
        )

        self.assertIsNone(result)
        self.assertEqual(len(logs), 1)
        self.assertIn("kind=input", logs[0])
        self.assertIn("error_type=CodexAppServerTransportError", logs[0])


if __name__ == "__main__":
    _ = unittest.main()

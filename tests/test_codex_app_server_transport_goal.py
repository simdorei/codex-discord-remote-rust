from __future__ import annotations

import unittest
from unittest import mock

import codex_app_server_transport as transport
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject


class PersistentCodexAppServerGoalTests(unittest.TestCase):
    def test_get_thread_goal_status_returns_none_when_thread_has_no_goal(self) -> None:
        client = transport.PersistentCodexAppServer()
        with mock.patch.object(client, "request", return_value={"goal": None}) as request:
            status = client.get_thread_goal_status("thread-1")

        self.assertIsNone(status)
        request.assert_called_once_with(
            "thread/goal/get",
            {"threadId": "thread-1"},
            timeout_sec=8.0,
        )

    def test_get_thread_goal_status_returns_validated_native_status(self) -> None:
        client = transport.PersistentCodexAppServer()
        response: JsonObject = {
            "goal": {
                "threadId": "thread-1",
                "status": "complete",
            }
        }
        with mock.patch.object(client, "request", return_value=response):
            status = client.get_thread_goal_status("thread-1")

        self.assertIs(status, transport.ThreadGoalStatus.COMPLETE)

    def test_get_thread_goal_status_rejects_malformed_or_wrong_thread_goal(self) -> None:
        invalid_results: list[JsonObject] = [
            {"goal": []},
            {"goal": {"threadId": "other", "status": "complete"}},
            {"goal": {"threadId": "thread-1", "status": "mystery"}},
        ]
        for result in invalid_results:
            with self.subTest(result=result):
                client = transport.PersistentCodexAppServer()
                with mock.patch.object(client, "request", return_value=result):
                    with self.assertRaises(CodexAppServerTransportError):
                        _ = client.get_thread_goal_status("thread-1")


if __name__ == "__main__":
    _ = unittest.main()

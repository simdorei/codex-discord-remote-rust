from __future__ import annotations

import unittest
from unittest import mock

import codex_app_server_transport as transport
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject


class UnexpectedActiveTurnReadError(Exception):
    pass


class ActiveTurnReadTimeout(TimeoutError):
    pass


class PersistentCodexAppServerActiveTurnTests(unittest.TestCase):
    def test_get_active_turn_id_waits_twenty_seconds_for_large_thread_history(self) -> None:
        client = transport.PersistentCodexAppServer(executable_resolver=lambda: "unused")
        with mock.patch.object(
            client,
            "read_thread",
            return_value={"thread": {"turns": [{"id": "turn-1", "status": "inProgress"}]}},
        ) as read_thread:
            self.assertEqual(
                client.get_active_turn_id_or_raise("thread-1", expected_generation=3),
                "turn-1",
            )

        read_thread.assert_called_once_with(
            "thread-1",
            include_turns=True,
            timeout_sec=20.0,
            expected_generation=3,
        )

    def test_has_active_turn_uses_thread_status_without_turn_history(self) -> None:
        client = transport.PersistentCodexAppServer(executable_resolver=lambda: "unused")
        with mock.patch.object(
            client,
            "read_thread",
            return_value={"thread": {"id": "thread-1", "status": {"type": "active"}}},
        ) as read_thread:
            self.assertTrue(client.has_active_turn_or_raise("thread-1"))

        read_thread.assert_called_once_with("thread-1", include_turns=False)

    def test_has_active_turn_returns_false_for_ephemeral_idle_thread(self) -> None:
        client = transport.PersistentCodexAppServer(executable_resolver=lambda: "unused")
        with mock.patch.object(
            client,
            "read_thread",
            return_value={"thread": {"id": "thread-1", "ephemeral": True, "status": {"type": "idle"}}},
        ) as read_thread:
            self.assertFalse(client.has_active_turn_or_raise("thread-1"))

        read_thread.assert_called_once_with("thread-1", include_turns=False)

    def test_get_active_turn_id_logs_transport_failure_and_returns_none(self) -> None:
        logs: list[str] = []
        client = transport.PersistentCodexAppServer(log_func=logs.append)

        def read_thread(thread_id: str, *, include_turns: bool = False, timeout_sec: float) -> JsonObject:
            self.assertEqual(thread_id, "thread-1")
            self.assertTrue(include_turns)
            self.assertEqual(timeout_sec, 20.0)
            raise CodexAppServerTransportError("transport down")

        with mock.patch.object(client, "read_thread", read_thread):
            self.assertIsNone(client.get_active_turn_id("thread-1"))

        self.assertEqual(len(logs), 1)
        self.assertIn("app_server_active_turn_read_failed thread=thread-1", logs[0])
        self.assertIn("error_type=CodexAppServerTransportError", logs[0])
        self.assertIn("transport down", logs[0])

    def test_get_active_turn_id_logs_timeout_failure_and_returns_none(self) -> None:
        logs: list[str] = []
        client = transport.PersistentCodexAppServer(log_func=logs.append)

        def read_thread(thread_id: str, *, include_turns: bool = False, timeout_sec: float) -> JsonObject:
            _ = (thread_id, include_turns, timeout_sec)
            raise ActiveTurnReadTimeout("read timed out")

        with mock.patch.object(client, "read_thread", read_thread):
            self.assertIsNone(client.get_active_turn_id("thread-1"))

        self.assertEqual(len(logs), 1)
        self.assertIn("error_type=ActiveTurnReadTimeout", logs[0])

    def test_get_active_turn_id_or_raise_surfaces_transport_failure(self) -> None:
        logs: list[str] = []
        client = transport.PersistentCodexAppServer(log_func=logs.append)

        def read_thread(thread_id: str, *, include_turns: bool = False, timeout_sec: float) -> JsonObject:
            _ = (thread_id, include_turns, timeout_sec)
            raise CodexAppServerTransportError("transport down")

        with mock.patch.object(client, "read_thread", read_thread):
            with self.assertRaisesRegex(CodexAppServerTransportError, "transport down"):
                _ = client.get_active_turn_id_or_raise("thread-1")

        self.assertEqual(logs, [])

    def test_get_active_turn_id_surfaces_unexpected_read_failure(self) -> None:
        logs: list[str] = []
        client = transport.PersistentCodexAppServer(log_func=logs.append)

        def read_thread(thread_id: str, *, include_turns: bool = False, timeout_sec: float) -> JsonObject:
            _ = (thread_id, include_turns, timeout_sec)
            raise UnexpectedActiveTurnReadError("boom")

        with mock.patch.object(client, "read_thread", read_thread):
            with self.assertRaisesRegex(UnexpectedActiveTurnReadError, "boom"):
                _ = client.get_active_turn_id("thread-1")

        self.assertEqual(logs, [])


if __name__ == "__main__":
    _ = unittest.main()

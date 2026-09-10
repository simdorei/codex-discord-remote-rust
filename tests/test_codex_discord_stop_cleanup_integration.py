from __future__ import annotations

import json
import unittest
from collections.abc import Callable
from unittest.mock import patch

import codex_app_server_transport as app_server_transport
import codex_discord_bot as bot
import codex_discord_bridge_command_runtime as bridge_command_runtime
from codex_app_server_transport_goal import GoalAbsent
from codex_app_server_transport_replies import JsonMapping, JsonObject
from codex_app_server_transport_subscriptions import ThreadReleaseStatus


def _record_write(writes: list[JsonObject]) -> Callable[[JsonMapping], None]:
    def write(payload: JsonMapping) -> None:
        writes.append(dict(payload))

    return write


class DiscordStopCleanupIntegrationTests(unittest.TestCase):
    def test_successful_stop_cancels_only_target_thread_pending_requests(self) -> None:
        # Given
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []
        client._handle_raw_line(
            json.dumps(
                {
                    "id": 17,
                    "method": "item/commandExecution/requestApproval",
                    "params": {"threadId": "thread-1", "turnId": "turn-1"},
                }
            )
        )
        client._handle_raw_line(
            json.dumps(
                {
                    "id": "input-2",
                    "method": "item/tool/requestUserInput",
                    "params": {"threadId": "thread-2", "turnId": "turn-2"},
                }
            )
        )
        original_client = app_server_transport.DEFAULT_CLIENT

        # When
        try:
            app_server_transport.DEFAULT_CLIENT = client
            with (
                patch.object(client, "_write_message", side_effect=_record_write(writes)),
                patch.object(bridge_command_runtime.bridge_process, "run_bridge_command", return_value=(0, "stopped")),
            ):
                result = bot.run_bridge_command(["stop", "--thread-id", "thread-1"])
        finally:
            app_server_transport.DEFAULT_CLIENT = original_client

        # Then
        self.assertEqual(result, (0, "stopped"))
        self.assertEqual(client.get_pending_server_requests("thread-1"), [])
        self.assertEqual(
            [request["id"] for request in client.get_pending_server_requests("thread-2")],
            ["input-2"],
        )
        self.assertEqual(
            writes,
            [
                {
                    "id": 17,
                    "error": {
                        "code": -32800,
                        "message": "Request cancelled because the remote operation ended.",
                    },
                }
            ],
        )

    def test_terminal_release_cancels_stale_pending_request_before_unsubscribe(self) -> None:
        # Given
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []
        client._handle_raw_line(
            json.dumps(
                {
                    "id": 23,
                    "method": "item/tool/requestUserInput",
                    "params": {"threadId": "thread-1", "turnId": "turn-1"},
                }
            )
        )
        client.mark_thread_subscribed("thread-1")

        # When
        with (
            patch.object(client, "has_active_turn_or_raise", return_value=False),
            patch.object(client, "get_thread_goal_lookup", return_value=GoalAbsent()),
            patch.object(client, "request", return_value={}),
            patch.object(client, "_write_message", side_effect=_record_write(writes)),
        ):
            outcome = client.release_thread_subscription_if_terminal("thread-1")

        # Then
        self.assertEqual(outcome.status, ThreadReleaseStatus.RELEASED)
        self.assertEqual(client.get_pending_server_requests("thread-1"), [])
        self.assertEqual(
            writes,
            [
                {
                    "id": 23,
                    "error": {
                        "code": -32800,
                        "message": "Request cancelled because the remote operation ended.",
                    },
                }
            ],
        )

    def test_terminal_release_preserves_pending_request_while_turn_is_active(self) -> None:
        # Given
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []
        client._handle_raw_line(
            json.dumps(
                {
                    "id": 29,
                    "method": "item/tool/requestUserInput",
                    "params": {"threadId": "thread-1", "turnId": "turn-1"},
                }
            )
        )
        client.mark_thread_subscribed("thread-1")

        # When
        with (
            patch.object(client, "has_active_turn_or_raise", return_value=True),
            patch.object(client, "_write_message", side_effect=_record_write(writes)),
        ):
            outcome = client.release_thread_subscription_if_terminal("thread-1")

        # Then
        self.assertEqual(outcome.status, ThreadReleaseStatus.ACTIVE_TURN)
        self.assertEqual(
            [request["id"] for request in client.get_pending_server_requests("thread-1")],
            [29],
        )
        self.assertEqual(writes, [])


if __name__ == "__main__":
    _ = unittest.main()

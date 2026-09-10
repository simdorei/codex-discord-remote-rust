from __future__ import annotations

# pyright: reportPrivateUsage=false

from collections.abc import Callable
import json
from typing import override
import unittest
from unittest.mock import patch

import codex_app_server_transport as app_server_transport
import codex_discord_bot as bot
from codex_app_server_transport_replies import JsonMapping, JsonObject
from codex_thread_models import ThreadInfo

from tests.test_codex_discord_bot import EnvPatch


class PendingInputUnavailable(RuntimeError):
    @override
    def __str__(self) -> str:
        return "No pending user input request is active for the selected thread."


def record_app_server_write(writes: list[JsonObject]) -> Callable[[JsonMapping], None]:
    def write(payload: JsonMapping) -> None:
        writes.append(dict(payload))

    return write


class DiscordAppServerPendingRepliesIntegrationTests(unittest.TestCase):
    def test_app_server_transport_replies_to_pending_approval_request(self) -> None:
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []

        with (
            patch.object(client, "start", return_value=None),
            patch.object(client, "_is_running", return_value=True),
            patch.object(client, "_write_message", side_effect=record_app_server_write(writes)),
        ):
            client._handle_raw_line(
                json.dumps(
                    {
                        "id": "approval-1",
                        "method": "item/commandExecution/requestApproval",
                        "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": "item-1"},
                    }
                )
            )

            result = client.reply_to_pending_approval("thread-1", "2")

        self.assertEqual(result["decision_action"], "acceptForSession")
        self.assertEqual(writes, [{"id": "approval-1", "result": {"decision": "acceptForSession"}}])
        self.assertEqual(client.get_pending_server_requests("thread-1"), [])

    def test_app_server_transport_replies_to_pending_input_request(self) -> None:
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []

        with (
            patch.object(client, "start", return_value=None),
            patch.object(client, "_is_running", return_value=True),
            patch.object(client, "_write_message", side_effect=record_app_server_write(writes)),
        ):
            client._handle_raw_line(
                json.dumps(
                    {
                        "id": "input-1",
                        "method": "item/tool/requestUserInput",
                        "params": {
                            "threadId": "thread-1",
                            "turnId": "turn-1",
                            "itemId": "item-1",
                            "questions": [
                                {
                                    "id": "mode",
                                    "header": "Mode",
                                    "question": "Pick mode",
                                    "isOther": False,
                                    "isSecret": False,
                                    "options": [
                                        {"label": "Fast", "description": ""},
                                        {"label": "Careful", "description": ""},
                                    ],
                                }
                            ],
                        },
                    }
                )
            )

            result = client.reply_to_pending_input("thread-1", "2")

        self.assertEqual(result["answers_by_question"], {"mode": ["Careful"]})
        self.assertEqual(
            writes,
            [{"id": "input-1", "result": {"answers": {"mode": {"answers": ["Careful"]}}}}],
        )
        self.assertEqual(client.get_pending_server_requests("thread-1"), [])

    def test_submit_approval_reply_prefers_resident_app_server_request(self) -> None:
        original_client = app_server_transport.DEFAULT_CLIENT
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []

        with (
            patch.object(client, "start", return_value=None),
            patch.object(client, "_is_running", return_value=True),
            patch.object(client, "_write_message", side_effect=record_app_server_write(writes)),
        ):
            client._handle_raw_line(
                json.dumps(
                    {
                        "id": "approval-1",
                        "method": "item/fileChange/requestApproval",
                        "params": {"threadId": "thread-1", "turnId": "turn-1", "itemId": "item-1"},
                    }
                )
            )
            try:
                app_server_transport.DEFAULT_CLIENT = client
                with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                    exit_code, output = bot.submit_approval_reply("thread-1", "1")
            finally:
                app_server_transport.DEFAULT_CLIENT = original_client

        self.assertEqual(exit_code, 0)
        self.assertIn("transport: resident-app-server approval", output)
        self.assertEqual(writes, [{"id": "approval-1", "result": {"decision": "accept"}}])

    def test_submit_input_reply_prefers_resident_app_server_request(self) -> None:
        original_client = app_server_transport.DEFAULT_CLIENT
        client = app_server_transport.PersistentCodexAppServer()
        writes: list[JsonObject] = []

        with (
            patch.object(client, "start", return_value=None),
            patch.object(client, "_is_running", return_value=True),
            patch.object(client, "_write_message", side_effect=record_app_server_write(writes)),
        ):
            client._handle_raw_line(
                json.dumps(
                    {
                        "id": "input-1",
                        "method": "item/tool/requestUserInput",
                        "params": {
                            "threadId": "thread-1",
                            "turnId": "turn-1",
                            "itemId": "item-1",
                            "questions": [{"id": "answer", "options": None}],
                        },
                    }
                )
            )
            try:
                app_server_transport.DEFAULT_CLIENT = client
                with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                    exit_code, output = bot.submit_input_reply("thread-1", "hello")
            finally:
                app_server_transport.DEFAULT_CLIENT = original_client

        self.assertEqual(exit_code, 0)
        self.assertIn("transport: resident-app-server input", output)
        self.assertEqual(
            writes,
            [{"id": "input-1", "result": {"answers": {"answer": {"answers": ["hello"]}}}}],
        )

    def test_submit_input_reply_legacy_fallback_surfaces_bridge_error(self) -> None:
        original_choose_thread = bot.BRIDGE_PENDING_INPUT_REPLY.choose_thread

        def raise_unavailable(
            thread_id: str | None,
            cwd: str | None = None,
        ) -> ThreadInfo:
            _ = thread_id, cwd
            raise PendingInputUnavailable()

        try:
            bot.BRIDGE_PENDING_INPUT_REPLY.choose_thread = raise_unavailable
            with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "0"):
                exit_code, output = bot.submit_input_reply("thread-1", "hello")
        finally:
            bot.BRIDGE_PENDING_INPUT_REPLY.choose_thread = original_choose_thread

        self.assertEqual(exit_code, 1)
        self.assertEqual(
            output,
            "ERROR: No pending user input request is active for the selected thread.",
        )


if __name__ == "__main__":
    _ = unittest.main()

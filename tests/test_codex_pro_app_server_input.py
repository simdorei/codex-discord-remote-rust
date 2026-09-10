from __future__ import annotations

import unittest
from unittest import mock

import codex_app_server_transport as app_server_transport
from codex_pro_prompt_contract import PRO_SKILL_CALL


class ProAppServerInputTests(unittest.TestCase):
    def test_start_turn_attaches_pro_skill_and_browser_mention(self) -> None:
        client = app_server_transport.PersistentCodexAppServer()
        with mock.patch.object(client, "request", return_value={"turn": {"id": "turn-1"}}) as request:
            _ = client.start_turn("thread-1", PRO_SKILL_CALL)

        params = request.call_args.args[1]
        inputs = params["input"]
        self.assertEqual([item["type"] for item in inputs], ["text", "skill", "mention"])

    def test_steer_turn_attaches_pro_skill_and_browser_mention(self) -> None:
        client = app_server_transport.PersistentCodexAppServer()
        with mock.patch.object(client, "request", return_value={"turnId": "turn-1"}) as request:
            _ = client.steer_turn(
                "thread-1",
                PRO_SKILL_CALL,
                expected_turn_id="turn-1",
            )

        params = request.call_args.args[1]
        inputs = params["input"]
        self.assertEqual([item["type"] for item in inputs], ["text", "skill", "mention"])


if __name__ == "__main__":
    unittest.main()

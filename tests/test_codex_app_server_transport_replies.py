from __future__ import annotations

import unittest

import codex_app_server_transport as transport
import codex_app_server_transport_replies as replies


class CodexAppServerTransportReplyTests(unittest.TestCase):
    def test_command_approval_session_choice_builds_session_decision(self) -> None:
        payload, decision = transport.build_approval_response(
            "item/commandExecution/requestApproval",
            {},
            "2",
        )

        self.assertEqual(payload, {"decision": "acceptForSession"})
        self.assertEqual(decision, "acceptForSession")

    def test_permission_approval_without_permissions_uses_safe_default_scope(self) -> None:
        payload, decision = transport.build_approval_response(
            "item/permissions/requestApproval",
            {},
            "1",
        )

        self.assertEqual(payload, {"permissions": {"network": None, "fileSystem": None}, "scope": "turn"})
        self.assertEqual(decision, "accept")

    def test_unsupported_approval_method_raises_transport_error(self) -> None:
        with self.assertRaises(transport.CodexAppServerTransportError):
            _ = transport.build_approval_response("unknown/requestApproval", {}, "1")

    def test_mcp_elicitation_approval_builds_protocol_response(self) -> None:
        payload, decision = transport.build_approval_response(
            "mcpServer/elicitation/request",
            {"mode": "url"},
            "1",
        )

        self.assertEqual(payload, {"action": "accept", "content": {}, "_meta": None})
        self.assertEqual(decision, "accept")

    def test_single_question_numeric_input_resolves_option_label(self) -> None:
        payload, answers = transport.build_input_response(
            {
                "questions": [
                    {
                        "id": "mode",
                        "options": [
                            {"label": "Fast"},
                            {"label": "Careful"},
                        ],
                    }
                ]
            },
            "2",
        )

        self.assertEqual(answers, {"mode": ["Careful"]})
        self.assertEqual(payload, {"answers": {"mode": {"answers": ["Careful"]}}})

    def test_multi_question_assignments_map_all_question_ids(self) -> None:
        payload, answers = transport.build_input_response(
            {
                "questions": [
                    {"id": "mode", "options": [{"label": "Fast"}, {"label": "Careful"}]},
                    {"id": "confirm", "options": []},
                ]
            },
            "mode=Careful;confirm=yes",
        )

        self.assertEqual(answers, {"mode": ["Careful"], "confirm": ["yes"]})
        self.assertEqual(
            payload,
            {
                "answers": {
                    "mode": {"answers": ["Careful"]},
                    "confirm": {"answers": ["yes"]},
                }
            },
        )

    def test_unknown_question_id_raises_transport_error(self) -> None:
        with self.assertRaises(transport.CodexAppServerTransportError):
            _ = transport.build_input_response(
                {"questions": [{"id": "mode", "options": []}]},
                "mode=Careful;extra=yes",
            )

    def test_empty_input_answer_raises_transport_error(self) -> None:
        with self.assertRaises(transport.CodexAppServerTransportError):
            _ = transport.build_input_response(
                {"questions": [{"id": "mode", "options": []}]},
                " ",
            )

    def test_extract_response_result_returns_dict_result(self) -> None:
        result = replies.extract_response_result(
            "thread/read",
            {"result": {"thread": {"id": "thread-1"}}},
        )

        self.assertEqual(result, {"thread": {"id": "thread-1"}})

    def test_extract_response_result_raises_transport_error_message(self) -> None:
        with self.assertRaisesRegex(transport.CodexAppServerTransportError, "turn/start failed: busy"):
            _ = replies.extract_response_result(
                "turn/start",
                {"error": {"message": "busy"}},
            )

    def test_extract_response_result_handles_scalar_error(self) -> None:
        with self.assertRaisesRegex(transport.CodexAppServerTransportError, "turn/start failed: denied"):
            _ = replies.extract_response_result("turn/start", {"error": "denied"})

    def test_extract_response_result_rejects_invalid_result(self) -> None:
        with self.assertRaisesRegex(
            transport.CodexAppServerTransportError,
            "thread/read returned an invalid payload.",
        ):
            _ = replies.extract_response_result("thread/read", {"result": ["bad"]})

from __future__ import annotations

# pyright: reportUnknownMemberType=false, reportUnusedCallResult=false

import unittest

import codex_desktop_bridge as bridge
import codex_desktop_bridge_reply_payload as reply_payload


class ReplyPayloadTests(unittest.TestCase):
    def test_build_reply_input_payload_maps_single_and_multi_question_answers(self) -> None:
        single_request = _pending_request(
            [
                _question(
                    "choice",
                    [
                        {"label": "Approve", "description": ""},
                        {"label": "Deny", "description": ""},
                    ],
                )
            ]
        )

        payload, answers = reply_payload.build_reply_input_response_payload(
            single_request,
            "2",
        )

        self.assertEqual(payload, {"answers": {"choice": {"answers": ["Deny"]}}})
        self.assertEqual(answers, {"choice": ["Deny"]})

        multi_request = _pending_request(
            [
                _question("first", []),
                _question(
                    "second",
                    [
                        {"label": "One", "description": ""},
                        {"label": "Two", "description": ""},
                    ],
                ),
            ]
        )

        payload, answers = reply_payload.build_reply_input_response_payload(
            multi_request,
            "first=alpha|beta; second=2",
        )

        expected_answers = {
            "first": ["alpha", "beta"],
            "second": ["Two"],
        }
        self.assertEqual(
            payload,
            {
                "answers": {
                    "first": {"answers": ["alpha", "beta"]},
                    "second": {"answers": ["Two"]},
                }
            },
        )
        self.assertEqual(answers, expected_answers)
        self.assertEqual(
            bridge.build_reply_input_response_payload(multi_request, "first=alpha|beta; second=2"),
            (payload, answers),
        )

    def test_approval_decision_payload_normalizes_supported_replies(self) -> None:
        self.assertEqual(reply_payload.build_approval_decision_payload("1"), ("accept", "accept"))
        self.assertEqual(
            reply_payload.build_approval_decision_payload("2"),
            ("acceptForSession", "acceptForSession"),
        )
        self.assertEqual(reply_payload.build_approval_decision_payload("3"), ("decline", "decline"))
        self.assertEqual(reply_payload.build_approval_decision_payload("cancel"), ("cancel", "cancel"))
        self.assertEqual(bridge.build_approval_decision_payload("예"), ("accept", "accept"))
        self.assertEqual(reply_payload.build_approval_decision_candidate_payloads("accept"), ["accept"])

    def test_reply_input_payload_rejects_malformed_answers(self) -> None:
        with self.assertRaisesRegex(reply_payload.ReplyInputAnswerEmptyError, "Answer text was empty"):
            _ = reply_payload.build_reply_input_response_payload(_pending_request([_question("q1", [])]), "")

        with self.assertRaisesRegex(reply_payload.ReplyInputQuestionsMissingError, "No pending input questions"):
            _ = reply_payload.build_reply_input_response_payload(_pending_request([]), "x")

        with self.assertRaisesRegex(reply_payload.ReplyInputQuestionIdMissingError, "did not include an id"):
            _ = reply_payload.build_reply_input_response_payload(_pending_request([_question("", [])]), "x")

        with self.assertRaisesRegex(reply_payload.ReplyInputAssignmentFormatError, "Multi-question replies"):
            _ = reply_payload.build_reply_input_response_payload(
                _pending_request([_question("first", []), _question("second", [])]),
                "alpha",
            )

        with self.assertRaisesRegex(reply_payload.ReplyInputAssignmentQuestionIdMissingError, "missing the question id"):
            _ = reply_payload.build_reply_input_response_payload(
                _pending_request([_question("first", []), _question("second", [])]),
                "=alpha; second=beta",
            )

        with self.assertRaisesRegex(reply_payload.ReplyInputMissingAnswersError, "Missing answers for question ids: second"):
            _ = reply_payload.build_reply_input_response_payload(
                _pending_request([_question("first", []), _question("second", [])]),
                "first=alpha",
            )

        with self.assertRaisesRegex(reply_payload.ReplyInputUnknownQuestionError, "Unknown question ids: extra"):
            _ = reply_payload.build_reply_input_response_payload(
                _pending_request([_question("first", [])]),
                "first=alpha; extra=beta",
            )

        with self.assertRaisesRegex(reply_payload.ApprovalReplyUnrecognizedError, "Unrecognized approval reply"):
            _ = reply_payload.build_approval_decision_payload("maybe")


def _pending_request(questions: list[reply_payload.JsonObject]) -> reply_payload.JsonObject:
    question_values: list[reply_payload.JsonValue] = []
    for question in questions:
        question_values.append(question)
    return {"questions": question_values}


def _question(question_id: str, options: list[reply_payload.JsonObject]) -> reply_payload.JsonObject:
    option_values: list[reply_payload.JsonValue] = []
    for option in options:
        option_values.append(option)
    return {
        "id": question_id,
        "header": "Header",
        "question": "Question?",
        "is_other": False,
        "is_secret": False,
        "options": option_values,
    }


if __name__ == "__main__":
    unittest.main()

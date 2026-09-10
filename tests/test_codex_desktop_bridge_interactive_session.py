# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import codex_desktop_bridge as bridge
import codex_desktop_bridge_interactive_session as interactive_session


class InteractiveSessionHappyTests(unittest.TestCase):
    def test_reads_pending_input_approval_display_and_messages(self) -> None:
        input_payload = _function_call(
            name="request_user_input",
            call_id="input-1",
            arguments={
                "questions": [
                    {
                        "question": "Choose mode?",
                        "options": [{"label": "Fast"}, {"label": "Careful"}],
                    }
                ]
            },
        )
        approval_payload = _function_call(
            name="functions.exec_command",
            call_id="approval-1",
            arguments={"sandbox_permissions": "require_escalated", "justification": "Need admin"},
        )

        self.assertEqual(interactive_session.classify_interactive_function_call(input_payload), "waiting-input")
        self.assertEqual(interactive_session.classify_interactive_function_call(approval_payload), "waiting-approval")
        self.assertEqual(
            interactive_session.build_interactive_notice_from_function_call(input_payload).splitlines(),
            ["[choice_required]", "Choose mode?", "1. Fast", "2. Careful"],
        )
        self.assertEqual(
            interactive_session.build_interactive_notice_from_function_call(approval_payload).splitlines(),
            ["[approval_required]", "tool: functions.exec_command", "Need admin"],
        )

        session_path = _session_file(
            _message("user", "hello"),
            _message("assistant", "hi"),
            {"type": "response_item", "payload": input_payload},
            {"type": "response_item", "payload": _function_output("input-1")},
            {"type": "response_item", "payload": approval_payload},
        )

        self.assertEqual(interactive_session.get_pending_interactive_state_from_session(session_path), "waiting-approval")
        self.assertEqual(
            interactive_session.get_pending_permission_approval_from_session(session_path),
            {"call_id": "approval-1", "tool_name": "functions.exec_command", "question": "Need admin"},
        )
        self.assertEqual(
            interactive_session.get_pending_interactive_display_lines(session_path),
            ("waiting-approval", ["tool: functions.exec_command", "Need admin"]),
        )
        self.assertEqual(
            interactive_session.get_pending_interactive_summary(session_path),
            "tool: functions.exec_command | Need admin",
        )
        self.assertEqual(interactive_session.get_last_user_and_assistant_messages(session_path), ("hello", "hi"))

        self.assertEqual(bridge.get_pending_interactive_state_from_session(session_path), "waiting-approval")
        bridge_permission = bridge.get_pending_permission_approval_from_session(session_path)
        self.assertIsNotNone(bridge_permission)
        if bridge_permission is not None:
            self.assertEqual(bridge_permission["call_id"], "approval-1")
        self.assertEqual(bridge.get_last_user_and_assistant_messages(session_path), ("hello", "hi"))


class InteractiveSessionEdgeTests(unittest.TestCase):
    def test_malformed_and_completed_calls_are_ignored(self) -> None:
        self.assertEqual(interactive_session.parse_function_call_arguments({"arguments": {"ok": True}}), {"ok": True})
        self.assertEqual(interactive_session.parse_function_call_arguments({"arguments": "{not-json"}), {})
        self.assertEqual(interactive_session.parse_function_call_arguments({"arguments": ["bad"]}), {})
        self.assertIsNone(interactive_session.classify_interactive_function_call({"type": "function_call", "name": "noop"}))

        completed_path = _session_file(
            {"type": "response_item", "payload": _function_call(name="request_user_input", call_id="done-1")},
            {"type": "response_item", "payload": _function_output("done-1")},
        )
        self.assertIsNone(interactive_session.get_pending_interactive_function_call_from_session(completed_path))

        missing_path = Path(tempfile.gettempdir()) / "codex-missing-session.jsonl"
        self.assertIsNone(interactive_session.get_pending_interactive_function_call_from_session(missing_path))
        self.assertEqual(interactive_session.get_pending_interactive_display_lines(missing_path), (None, []))
        self.assertEqual(interactive_session.summarize_interactive_lines(None, []), "")
        self.assertEqual(interactive_session.summarize_interactive_lines("waiting-input", []), "")

        malformed_messages = _session_file(
            {"type": "not-json", "payload": []},
            {"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "unknown"}]}},
            {"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]}},
        )
        self.assertEqual(interactive_session.get_last_user_and_assistant_messages(malformed_messages), ("", "ok"))


def _function_call(
    *,
    name: str,
    call_id: str = "call-1",
    arguments: interactive_session.JsonObject | None = None,
) -> interactive_session.JsonObject:
    return {
        "type": "function_call",
        "name": name,
        "call_id": call_id,
        "arguments": json.dumps(arguments or {}),
    }


def _function_output(call_id: str) -> interactive_session.JsonObject:
    return {"type": "function_call_output", "call_id": call_id, "output": "ok"}


def _message(role: str, text: str) -> interactive_session.JsonObject:
    return {
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": role,
            "content": [{"type": "input_text" if role == "user" else "output_text", "text": text}],
        },
    }


def _session_file(*events: interactive_session.JsonObject) -> Path:
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".jsonl", delete=False)
    with handle:
        for event in events:
            handle.write(json.dumps(event) + "\n")
    return Path(handle.name)


if __name__ == "__main__":
    unittest.main()

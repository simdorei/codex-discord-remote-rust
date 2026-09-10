# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import unittest
from pathlib import Path

import codex_desktop_bridge as bridge
import codex_desktop_bridge_reply as reply
from codex_thread_models import ThreadInfo


class PendingReplyTests(unittest.TestCase):
    def test_user_input_happy_path_returns_answers(self) -> None:
        submitted: list[reply.JsonObject] = []
        deps = _deps(
            input_request={"request_id": "input-1"},
            build_input=lambda _request, _answer: ({"answers": {}}, {"mode": ["Fast"]}),
            submit_user=lambda _thread, _request_id, payload, _timeout: submitted.append(payload) or {"ok": True},
        )

        result = reply.reply_to_pending_user_input(_thread(), "1", 1.0, deps)

        self.assertEqual(result["request_id"], "input-1")
        self.assertEqual(result["answers_by_question"], {"mode": ["Fast"]})
        self.assertEqual(submitted, [{"answers": {}}])

    def test_approval_happy_path_attempts_target_modes_and_clears_cache(self) -> None:
        calls: list[tuple[reply.JsonValue, reply.JsonValue, bool]] = []
        cleared: list[str] = []
        states = iter(["waiting-approval", "idle"])
        deps = _deps(
            approval_request={"request_id": "approval-1", "request_kind": "fileChange", "owner_client_id": "owner"},
            approval_candidates=lambda _payload: ["accept"],
            submit_approval=lambda _thread, request_id, payload, _kind, _timeout, use_target: calls.append(
                (request_id, payload, use_target)
            )
            or {"owner_client_id": "owner"},
            busy=lambda _thread, _allow_resume: next(states),
            clear_cached=cleared.append,
        )

        result = reply.reply_to_pending_approval(_thread(), "yes", 1.0, deps)

        self.assertEqual(calls, [("approval-1", "accept", True), ("approval-1", "accept", False)])
        self.assertEqual(cleared, ["thread-1"])
        self.assertEqual(result["decision_action"], "accept")
        self.assertEqual(result["request_kind"], "fileChange")
        self.assertEqual(result["verification_busy_state"], "idle")
        self.assertEqual(result["attempts"], ["payload#1=string/target=owner", "payload#1=string/target=broadcast"])

    def test_permission_fallback_preserves_verification(self) -> None:
        states = iter(["waiting-approval", "idle"])
        deps = _deps(
            approval_request=None,
            busy=lambda _thread, _allow_resume: next(states),
            permission_prompt=lambda _path: {"call_id": "call-1", "tool_name": "tool"},
            submit_permission=lambda answer: {"answer": answer},
            now=_counter([0.0, 0.0, 9.0]),
            sleep=lambda _seconds: None,
        )

        result = reply.reply_to_pending_approval(_thread(), "2", 1.0, deps)

        self.assertEqual(result["answer"], "2")
        self.assertEqual(result["request_id"], "call-1")
        self.assertEqual(result["verification_busy_state"], "idle")

    def test_bridge_wrappers_delegate_to_reply_module(self) -> None:
        original_user = bridge.get_pending_user_input_request_via_ipc
        original_payload = bridge.build_reply_input_response_payload
        original_submit = bridge.submit_user_input_via_ipc
        try:
            bridge.get_pending_user_input_request_via_ipc = lambda _thread, timeout_sec=6.0: {"request_id": "input-1"}
            bridge.build_reply_input_response_payload = lambda _request, _answer: (
                {"answers": {}},
                {"mode": ["Careful"]},
            )
            bridge.submit_user_input_via_ipc = lambda _thread, _request_id, _payload, timeout_sec=6.0: {}

            result = bridge.reply_to_pending_user_input(_thread(), "2")
        finally:
            bridge.get_pending_user_input_request_via_ipc = original_user
            bridge.build_reply_input_response_payload = original_payload
            bridge.submit_user_input_via_ipc = original_submit

        self.assertEqual(result["request_id"], "input-1")
        self.assertEqual(result["answers_by_question"], {"mode": ["Careful"]})

    def test_missing_snapshot_and_invalid_request_edges(self) -> None:
        with self.assertRaisesRegex(reply.PendingUserInputMissingError, "No pending user input request"):
            _ = reply.reply_to_pending_user_input(_thread(), "x", 1.0, _deps(input_request=None, busy=_busy("idle")))

        with self.assertRaisesRegex(reply.PendingUserInputSnapshotMissingError, "waiting-input"):
            _ = reply.reply_to_pending_user_input(
                _thread(),
                "x",
                1.0,
                _deps(input_request=None, busy=_busy("waiting-input")),
            )

        with self.assertRaisesRegex(reply.PendingUserInputRequestIdMissingError, "pending input request did not include a request id"):
            _ = reply.reply_to_pending_user_input(_thread(), "x", 1.0, _deps(input_request={"request_id": ""}))

        with self.assertRaisesRegex(reply.PendingApprovalMissingError, "No pending approval request"):
            _ = reply.reply_to_pending_approval(_thread(), "yes", 1.0, _deps(approval_request=None, busy=_busy("idle")))

        with self.assertRaisesRegex(reply.PendingApprovalRequestIdMissingError, "pending approval request did not include a request id"):
            _ = reply.reply_to_pending_approval(
                _thread(),
                "yes",
                1.0,
                _deps(approval_request={"request_id": "", "request_kind": "fileChange"}),
            )

        with self.assertRaisesRegex(reply.PendingApprovalRequestKindMissingError, "pending approval request did not include a request kind"):
            _ = reply.reply_to_pending_approval(
                _thread(),
                "yes",
                1.0,
                _deps(approval_request={"request_id": "approval-1", "request_kind": ""}),
            )

        with self.assertRaisesRegex(reply.PendingApprovalSnapshotMissingError, "waiting-approval"):
            _ = reply.reply_to_pending_approval(
                _thread(),
                "yes",
                1.0,
                _deps(approval_request=None, busy=_busy("waiting-approval")),
            )

    def test_still_waiting_approval_failure_includes_attempts(self) -> None:
        deps = _deps(
            approval_request={"request_id": "approval-1", "request_kind": "commandExecution", "owner_client_id": "owner"},
            submit_approval=lambda *_args: {"owner_client_id": "owner"},
            busy=_busy("waiting-approval"),
        )

        with self.assertRaises(RuntimeError) as raised:
            _ = reply.reply_to_pending_approval(_thread(), "yes", 1.0, deps)

        message = str(raised.exception)
        self.assertIn("Approval submit was acknowledged", message)
        self.assertIn("payload#1=string/target=owner", message)
        self.assertIn("payload#1=string/target=broadcast", message)
        self.assertIn("handled_by_client: owner", message)


def _deps(
    *,
    input_request: reply.JsonObject | None = None,
    approval_request: reply.JsonObject | None = None,
    busy: reply.GetBusyState | None = None,
    build_input: reply.BuildInputPayload | None = None,
    submit_user: reply.SubmitUserInput | None = None,
    approval_candidates: reply.BuildApprovalCandidates | None = None,
    submit_approval: reply.SubmitApprovalDecision | None = None,
    clear_cached: reply.ClearCachedApproval | None = None,
    permission_prompt: reply.GetPermissionApproval | None = None,
    submit_permission: reply.SubmitPermissionApproval | None = None,
    now: reply.TimeNow | None = None,
    sleep: reply.Sleep | None = None,
) -> reply.PendingReplyDeps:
    return reply.PendingReplyDeps(
        get_pending_user_input_request=lambda _thread, _timeout: input_request,
        get_pending_approval_request=lambda _thread, _timeout: approval_request,
        get_thread_busy_state=busy or _busy("idle"),
        build_reply_input_response_payload=build_input or (lambda _request, _answer: ({"answers": {}}, {})),
        submit_user_input=submit_user or (lambda _thread, _request_id, _payload, _timeout: {}),
        get_cached_live_approval_request=lambda _thread_id: None,
        get_pending_permission_approval_from_session=permission_prompt or (lambda _path: None),
        submit_permission_approval_via_ui_row_select=submit_permission or (lambda _answer: {}),
        build_approval_decision_payload=lambda _answer: ("accept", "accept"),
        build_approval_decision_candidate_payloads=approval_candidates or (lambda payload: [payload]),
        submit_approval_decision=submit_approval or (lambda *_args: {}),
        clear_cached_live_approval_request=clear_cached or (lambda _thread_id: None),
        time_now=now or (lambda: 0.0),
        sleep=sleep or (lambda _seconds: None),
    )


def _busy(state: str) -> reply.GetBusyState:
    return lambda _thread, _allow_resume: state


def _counter(values: list[float]) -> reply.TimeNow:
    remaining = iter(values)
    return lambda: next(remaining)


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path=str(Path("session.jsonl")),
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


if __name__ == "__main__":
    unittest.main()

# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateLocalImportUsage=false, reportPrivateUsage=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import unittest

import codex_desktop_bridge as bridge
import codex_desktop_bridge_ipc_submit as ipc_submit
import codex_desktop_bridge_ipc_submit_requests as submit_requests
from codex_thread_models import ThreadInfo


class IpcSubmitTests(unittest.TestCase):
    def test_submit_user_input_builds_payload_and_owner_fallback(self) -> None:
        written_payloads: list[ipc_submit.JsonObject] = []
        deps = _deps_for_response(
            {"resultType": "success", "handledByClientId": "handled-client", "result": {"ok": True}},
            written_payloads,
        )

        result = ipc_submit.request_submit_user_input_via_ipc(
            handle=1,
            source_client_id="source-client",
            thread_id="thread-1",
            request_id="input-1",
            response_payload={"answers": {"q1": {"answers": ["yes"]}}},
            timeout_sec=1.0,
            owner_clients={"thread-1": "owner-client"},
            deps=deps,
        )

        self.assertEqual(result, {"ok": True, "owner_client_id": "handled-client"})
        request = written_payloads[0]
        self.assertEqual(request["method"], "thread-follower-submit-user-input")
        self.assertEqual(request["targetClientId"], "owner-client")
        params = _require_json_object(request.get("params"))
        self.assertEqual(params["conversationId"], "thread-1")
        self.assertEqual(params["requestId"], "input-1")
        self.assertEqual(params["response"], {"answers": {"q1": {"answers": ["yes"]}}})

    def test_submit_approval_builds_payload_and_method_mapping(self) -> None:
        written_payloads: list[ipc_submit.JsonObject] = []
        deps = _deps_for_response({"resultType": "success", "result": {}}, written_payloads)

        result = ipc_submit.request_submit_approval_decision_via_ipc(
            handle=1,
            source_client_id="source-client",
            thread_id="thread-1",
            request_id="approval-1",
            decision_payload="accept",
            timeout_sec=1.0,
            owner_clients={"thread-1": "owner-client"},
            method=ipc_submit.approval_decision_method_for_request_kind("commandExecution"),
            deps=deps,
        )

        self.assertEqual(result, {"owner_client_id": "owner-client"})
        request = written_payloads[0]
        self.assertEqual(request["method"], "thread-follower-command-approval-decision")
        self.assertEqual(request["targetClientId"], "owner-client")
        params = _require_json_object(request.get("params"))
        self.assertEqual(params["conversationId"], "thread-1")
        self.assertEqual(params["requestId"], "approval-1")
        self.assertEqual(params["decision"], "accept")
        self.assertEqual(
            ipc_submit.approval_decision_method_for_request_kind("fileChange"),
            "thread-follower-file-approval-decision",
        )

    def test_bridge_public_submit_wrappers_preserve_defaults(self) -> None:
        thread = _thread()
        win32 = bridge.kernel32
        if win32 is None:
            self.fail("Windows kernel32 is required for this IPC wrapper test.")
        original_open = bridge._open_codex_ipc_pipe
        original_init = bridge._initialize_ipc_client
        original_discover = bridge._discover_owner_client_for_thread
        original_close = win32.CloseHandle
        original_user_input = bridge._request_submit_user_input_via_ipc
        original_approval = bridge._request_submit_approval_decision_via_ipc
        user_input_calls: list[ipc_submit.JsonObject] = []
        approval_calls: list[ipc_submit.JsonObject] = []

        def request_user_input(
            handle: int,
            source_client_id: str,
            thread_id: str,
            request_id: str,
            response_payload: ipc_submit.JsonObject,
            timeout_sec: float,
            owner_clients: dict[str, str],
        ) -> ipc_submit.JsonObject:
            _ = (handle, source_client_id, thread_id, response_payload, timeout_sec, owner_clients)
            user_input_calls.append({"request_id": request_id})
            return {}

        def request_approval(
            handle: int,
            source_client_id: str,
            thread_id: str,
            request_id: ipc_submit.JsonValue,
            decision_payload: ipc_submit.JsonValue,
            timeout_sec: float,
            owner_clients: dict[str, str],
            *,
            method: str,
            use_target_client: bool = True,
        ) -> ipc_submit.JsonObject:
            _ = (handle, source_client_id, thread_id, decision_payload, timeout_sec, owner_clients)
            approval_calls.append(
                {
                    "request_id": str(request_id),
                    "method": method,
                    "use_target_client": use_target_client,
                }
            )
            return {}

        try:
            bridge._open_codex_ipc_pipe = lambda: 10
            bridge._initialize_ipc_client = lambda *_args, **_kwargs: "source-client"
            bridge._discover_owner_client_for_thread = lambda *_args, **_kwargs: "owner-client"
            win32.CloseHandle = lambda _handle: None
            bridge._request_submit_user_input_via_ipc = request_user_input
            bridge._request_submit_approval_decision_via_ipc = request_approval

            user_result = bridge.submit_user_input_via_ipc(thread, "input-1", {"answers": {}})
            approval_result = bridge.submit_approval_decision_via_ipc(
                thread,
                "approval-1",
                "accept",
                "fileChange",
                use_target_client=False,
            )
        finally:
            bridge._open_codex_ipc_pipe = original_open
            bridge._initialize_ipc_client = original_init
            bridge._discover_owner_client_for_thread = original_discover
            win32.CloseHandle = original_close
            bridge._request_submit_user_input_via_ipc = original_user_input
            bridge._request_submit_approval_decision_via_ipc = original_approval

        self.assertEqual(user_result["request_id"], "input-1")
        self.assertEqual(approval_result["request_id"], "approval-1")
        self.assertEqual(approval_result["request_kind"], "fileChange")
        self.assertEqual(user_input_calls, [{"request_id": "input-1"}])
        self.assertEqual(
            approval_calls,
            [
                {
                    "request_id": "approval-1",
                    "method": "thread-follower-file-approval-decision",
                    "use_target_client": False,
                }
            ],
        )

    def test_submit_helpers_surface_transport_failures(self) -> None:
        with self.assertRaises(ipc_submit.IPCNoClientFoundError) as no_client:
            _ = ipc_submit.request_submit_user_input_via_ipc(
                handle=1,
                source_client_id="source-client",
                thread_id="thread-1",
                request_id="input-1",
                response_payload={"answers": {}},
                timeout_sec=1.0,
                owner_clients={},
                deps=_deps_for_response({"resultType": "error", "error": "no-client-found"}),
            )
        self.assertIn("no-client-found", str(no_client.exception))

        with self.assertRaises(ipc_submit.IPCSubmitResponseError) as failed:
            _ = ipc_submit.request_submit_approval_decision_via_ipc(
                handle=1,
                source_client_id="source-client",
                thread_id="thread-1",
                request_id="approval-1",
                decision_payload="accept",
                timeout_sec=1.0,
                owner_clients={},
                method="thread-follower-command-approval-decision",
                deps=_deps_for_response({"resultType": "error", "error": "boom"}),
            )
        self.assertIn("IPC approval decision failed: boom", str(failed.exception))

        with self.assertRaises(RuntimeError) as invalid:
            _ = ipc_submit.request_submit_user_input_via_ipc(
                handle=1,
                source_client_id="source-client",
                thread_id="thread-1",
                request_id="input-1",
                response_payload={"answers": {}},
                timeout_sec=1.0,
                owner_clients={},
                deps=_deps_for_response({"resultType": "success", "result": "bad"}),
            )
        self.assertIn("invalid payload", str(invalid.exception))

    def test_approval_use_target_client_and_unsupported_kind_edges(self) -> None:
        written_payloads: list[ipc_submit.JsonObject] = []
        deps = _deps_for_response({"resultType": "success", "result": {}}, written_payloads)
        _ = ipc_submit.request_submit_approval_decision_via_ipc(
            handle=1,
            source_client_id="source-client",
            thread_id="thread-1",
            request_id="approval-1",
            decision_payload="accept",
            timeout_sec=1.0,
            owner_clients={"thread-1": "owner-client"},
            method="thread-follower-command-approval-decision",
            deps=deps,
            use_target_client=False,
        )
        self.assertNotIn("targetClientId", written_payloads[0])

        original_open = bridge._open_codex_ipc_pipe
        try:
            bridge._open_codex_ipc_pipe = _fail_open
            with self.assertRaisesRegex(
                submit_requests.UnsupportedApprovalRequestKindError,
                "Unsupported approval request kind: other",
            ):
                _ = bridge.submit_approval_decision_via_ipc(_thread(), "approval-1", "accept", "other")
        finally:
            bridge._open_codex_ipc_pipe = original_open


def _deps_for_response(
    response: ipc_submit.JsonObject,
    written_payloads: list[ipc_submit.JsonObject] | None = None,
) -> ipc_submit.IpcSubmitDeps:
    def write_message(_handle: int, payload: ipc_submit.JsonObject) -> None:
        if written_payloads is not None:
            written_payloads.append(payload)

    def read_response(
        _handle: int,
        _request_id: str,
        _timeout_sec: float,
        _owner_clients: dict[str, str],
    ) -> ipc_submit.JsonObject:
        return response

    return ipc_submit.IpcSubmitDeps(
        write_ipc_message=write_message,
        read_ipc_response=read_response,
    )


def _require_json_object(value: ipc_submit.JsonValue | None) -> ipc_submit.JsonObject:
    if not isinstance(value, dict):
        raise AssertionError(f"Expected JSON object, got {value!r}")
    return value


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path="session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


def _fail_open() -> int:
    raise AssertionError("IPC should not open for unsupported approval request kind")


if __name__ == "__main__":
    unittest.main()

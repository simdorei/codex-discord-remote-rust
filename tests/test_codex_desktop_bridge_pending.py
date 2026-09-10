# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import unittest

import codex_desktop_bridge as bridge
import codex_desktop_bridge_pending as pending
from codex_thread_models import ThreadInfo


class PendingBridgeTests(unittest.TestCase):
    def test_extracts_pending_requests_and_display_lines(self) -> None:
        approval = pending.extract_pending_approval_request(
            {
                "requests": [
                    {"id": "old", "method": "item/fileChange/requestApproval", "params": {"threadId": "other"}},
                    {
                        "id": 7,
                        "method": "item/commandExecution/requestApproval",
                        "params": {"threadId": "thread-1", "itemId": "item-1", "reason": " allow command "},
                    },
                ]
            },
            "thread-1",
        )
        self.assertEqual(
            approval,
            {
                "thread_id": "thread-1",
                "request_id": 7,
                "request_kind": "commandExecution",
                "method": "item/commandExecution/requestApproval",
                "item_id": "item-1",
                "reason": "allow command",
            },
        )

        user_input = pending.extract_pending_user_input_request(_user_input_state(), "thread-1")
        self.assertIsNotNone(user_input)
        assert user_input is not None
        self.assertEqual(user_input["request_id"], "input-1")
        self.assertEqual(
            user_input["questions"],
            [
                {
                    "id": "mode",
                    "header": "Mode",
                    "question": "Pick",
                    "is_other": False,
                    "is_secret": True,
                    "options": [{"label": "Fast", "description": ""}],
                }
            ],
        )

        state, lines = pending.get_live_pending_approval_display_lines(
            _thread(),
            timeout_sec=0.5,
            reason_limit=8,
            get_pending_approval_request=lambda _thread, _timeout: {"request_kind": "fileChange", "reason": "abcdefghi"},
            collapse_list_text=lambda text, limit: text[:limit],
        )
        self.assertEqual(state, "waiting-approval")
        self.assertEqual(lines[:2], ["kind: fileChange", "abcdefgh"])
        self.assertIn("2. yes + do not ask again in this session", lines)

    def test_ipc_polling_caches_owner_and_preserves_bridge_wrappers(self) -> None:
        cached: list[pending.JsonObject] = []
        deps = _deps_for_snapshot(_approval_state(), cached, owner_client_id="owner-from-snapshot")

        result = pending.get_pending_approval_request_via_ipc(
            handle=1,
            thread=_thread(),
            timeout_sec=1.0,
            owner_clients={},
            pipe_peek_retry_sec=0.01,
            deps=deps,
        )

        self.assertIsNotNone(result)
        assert result is not None
        self.assertEqual(result["owner_client_id"], "owner-from-snapshot")
        self.assertEqual(cached, [result])

        result_without_owner = dict(result)
        _ = result_without_owner.pop("owner_client_id", None)
        self.assertEqual(bridge._extract_pending_approval_request(_approval_state(), "thread-1"), result_without_owner)
        self.assertEqual(
            bridge._extract_pending_user_input_request(_user_input_state(), "thread-1"),
            pending.extract_pending_user_input_request(_user_input_state(), "thread-1"),
        )

    def test_bridge_public_pending_ipc_wrappers_close_handle(self) -> None:
        win32 = bridge.kernel32
        if win32 is None:
            self.fail("Windows kernel32 is required for this IPC wrapper test.")
        original_open = bridge._open_codex_ipc_pipe
        original_init = bridge._initialize_ipc_client
        original_read = bridge._read_ipc_message
        original_record = bridge._record_owner_client_from_ipc_message
        original_snapshot = bridge._extract_thread_snapshot_from_ipc_message
        original_cache = bridge.cache_live_approval_request
        original_close = win32.CloseHandle
        cached: list[pending.JsonObject] = []
        closed: list[int] = []

        try:
            bridge._open_codex_ipc_pipe = lambda: 10
            bridge._initialize_ipc_client = lambda _handle, owner_clients, **_kwargs: owner_clients.setdefault(
                "thread-1", "owner-from-init"
            )
            bridge._read_ipc_message = lambda _handle, _timeout: {"type": "snapshot"}
            bridge._record_owner_client_from_ipc_message = lambda _message, _owners: None
            bridge._extract_thread_snapshot_from_ipc_message = lambda _message, _thread_id: (_approval_state(), "")
            bridge.cache_live_approval_request = lambda request: cached.append(request)
            win32.CloseHandle = lambda handle: closed.append(handle)

            result = bridge.get_pending_approval_request_via_ipc(_thread(), timeout_sec=1.0)
        finally:
            bridge._open_codex_ipc_pipe = original_open
            bridge._initialize_ipc_client = original_init
            bridge._read_ipc_message = original_read
            bridge._record_owner_client_from_ipc_message = original_record
            bridge._extract_thread_snapshot_from_ipc_message = original_snapshot
            bridge.cache_live_approval_request = original_cache
            win32.CloseHandle = original_close

        self.assertIsNotNone(result)
        assert result is not None
        self.assertEqual(result["owner_client_id"], "owner-from-init")
        self.assertEqual(cached, [result])
        self.assertEqual(closed, [10])

    def test_malformed_requests_and_polling_timeout_edges(self) -> None:
        self.assertIsNone(pending.extract_pending_approval_request({"requests": "bad"}, "thread-1"))
        self.assertIsNone(
            pending.extract_pending_approval_request(
                {
                    "requests": [
                        "bad",
                        {"id": "x", "method": "other", "params": {}},
                        {"id": "", "method": "item/fileChange/requestApproval", "params": {"threadId": "thread-1"}},
                        {
                            "id": "a",
                            "method": "item/fileChange/requestApproval",
                            "params": {"threadId": "other"},
                        },
                    ]
                },
                "thread-1",
            )
        )
        self.assertIsNone(
            pending.extract_pending_user_input_request(
                {"requests": [{"id": "input", "method": "item/tool/requestUserInput", "params": {"questions": []}}]},
                "thread-1",
            )
        )

        timeout_result = pending.get_pending_approval_request_via_ipc(
            handle=1,
            thread=_thread(),
            timeout_sec=1.0,
            owner_clients={},
            pipe_peek_retry_sec=0.01,
            deps=_timeout_deps(),
        )
        self.assertIsNone(timeout_result)

    def test_user_input_polling_stops_on_snapshot_without_request(self) -> None:
        deps = _deps_for_snapshot({"requests": []}, [], owner_client_id="")

        result = pending.get_pending_user_input_request_via_ipc(
            handle=1,
            thread=_thread(),
            timeout_sec=1.0,
            owner_clients={"thread-1": "owner"},
            pipe_peek_retry_sec=0.01,
            deps=deps,
        )

        self.assertIsNone(result)


def _deps_for_snapshot(
    conversation_state: pending.JsonObject,
    cached: list[pending.JsonObject],
    *,
    owner_client_id: str,
) -> pending.IpcPendingDeps:
    def read_message(_handle: int, _timeout_sec: float) -> pending.JsonObject:
        return {"type": "snapshot"}

    return pending.IpcPendingDeps(
        read_ipc_message=read_message,
        record_owner_client_from_ipc_message=lambda _message, _owners: None,
        extract_thread_snapshot_from_ipc_message=lambda _message, _thread_id: (conversation_state, owner_client_id),
        cache_live_approval_request=cached.append,
    )


def _timeout_deps() -> pending.IpcPendingDeps:
    def read_message(_handle: int, _timeout_sec: float) -> pending.JsonObject:
        raise TimeoutError

    return pending.IpcPendingDeps(
        read_ipc_message=read_message,
        record_owner_client_from_ipc_message=lambda _message, _owners: None,
        extract_thread_snapshot_from_ipc_message=lambda _message, _thread_id: None,
        cache_live_approval_request=lambda _request: None,
    )


def _approval_state() -> pending.JsonObject:
    return {
        "requests": [
            {
                "id": "approval-1",
                "method": "item/fileChange/requestApproval",
                "params": {"threadId": "thread-1", "itemId": "item-1", "reason": "edit file"},
            }
        ]
    }


def _user_input_state() -> pending.JsonObject:
    return {
        "requests": [
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
                            "question": "Pick",
                            "isSecret": True,
                            "options": [{"label": " Fast ", "description": ""}, {"label": "", "description": "bad"}],
                        }
                    ],
                },
            }
        ]
    }


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


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

from dataclasses import dataclass
import unittest

from codex_bridge_state import JsonObject, JsonValue
import codex_desktop_bridge_ipc_turn as ipc_turn
from codex_desktop_bridge_ipc_start_turn import IPCNoClientFoundError
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class IpcTurnHarness:
    deps: ipc_turn.IpcTurnDeps
    events: list[str]
    start_owner_snapshots: list[ipc_turn.OwnerClients]
    approval_calls: list[tuple[JsonValue, JsonValue, str, bool]]


class IpcTurnTests(unittest.TestCase):
    def test_start_turn_owner_missing_raises_typed_error_and_closes_pipe(self) -> None:
        harness = _make_harness(
            start_results=[
                IPCNoClientFoundError("no owner"),
                IPCNoClientFoundError("no owner"),
            ],
            discoveries=[None],
        )

        with self.assertRaises(ipc_turn.IPCOwnerClientMissingError) as raised:
            _ = ipc_turn.start_turn_via_ipc(
                _thread(),
                "hello",
                1.0,
                allow_ui_recovery=False,
                deps=harness.deps,
            )

        self.assertIn("IPC owner client for the selected thread was not discovered in background mode", str(raised.exception))
        self.assertIn("close:99", harness.events)

    def test_start_turn_ui_recovery_adds_recovery_method_after_owner_discovery(self) -> None:
        harness = _make_harness(
            start_results=[
                IPCNoClientFoundError("no owner"),
                {"turn_id": "turn-1"},
            ],
            discoveries=["owner-1"],
            activation_result="sidebar:Thread [header]",
        )

        result = ipc_turn.start_turn_via_ipc(
            _thread(),
            "hello",
            2.0,
            allow_ui_recovery=True,
            deps=harness.deps,
        )

        self.assertEqual(result, {"turn_id": "turn-1", "recovery_method": "sidebar:Thread [header]"})
        self.assertEqual(harness.start_owner_snapshots[1], {"thread-1": "owner-1"})
        self.assertIn("close:99", harness.events)

    def test_submit_approval_decision_preserves_json_payload_and_defaults(self) -> None:
        harness = _make_harness(start_results=[])

        result = ipc_turn.submit_approval_decision_via_ipc(
            _thread(),
            request_id=123,
            decision_payload={"decision": "accept"},
            request_kind="fileChange",
            timeout_sec=1.0,
            use_target_client=False,
            deps=harness.deps,
        )

        self.assertEqual(result, {"ok": True, "request_id": "123", "request_kind": "fileChange"})
        self.assertEqual(harness.approval_calls, [(123, {"decision": "accept"}, "method:fileChange", False)])
        self.assertIn("close:99", harness.events)


def _make_harness(
    *,
    start_results: list[dict[str, str] | BaseException],
    discoveries: list[str | None] | None = None,
    activation_result: str = "sidebar:Thread [header]",
) -> IpcTurnHarness:
    events: list[str] = []
    start_owner_snapshots: list[ipc_turn.OwnerClients] = []
    approval_calls: list[tuple[JsonValue, JsonValue, str, bool]] = []
    pending_start_results = list(start_results)
    pending_discoveries = list(discoveries or [])

    def open_ipc_pipe() -> int:
        events.append("open")
        return 99

    def close_ipc_pipe(handle: int) -> None:
        events.append(f"close:{handle}")

    def initialize_ipc_client(handle: int, owner_clients: ipc_turn.OwnerClients, timeout_sec: float) -> str:
        events.append(f"initialize:{handle}:{timeout_sec}")
        self_owner_clients = owner_clients
        _ = self_owner_clients
        return "source-client"

    def request_start_turn(
        handle: int,
        source_client_id: str,
        thread: ThreadInfo,
        prompt: str,
        timeout_sec: float,
        owner_clients: ipc_turn.OwnerClients,
    ) -> dict[str, str]:
        _ = (handle, source_client_id, thread, prompt, timeout_sec)
        start_owner_snapshots.append(dict(owner_clients))
        if not pending_start_results:
            raise AssertionError("unexpected start-turn request")
        next_result = pending_start_results.pop(0)
        if isinstance(next_result, BaseException):
            raise next_result
        return dict(next_result)

    def request_submit_user_input(
        handle: int,
        source_client_id: str,
        thread_id: str,
        request_id: str,
        response_payload: JsonObject,
        timeout_sec: float,
        owner_clients: ipc_turn.OwnerClients,
    ) -> JsonObject:
        _ = (handle, source_client_id, thread_id, request_id, response_payload, timeout_sec, owner_clients)
        return {"ok": True}

    def request_submit_approval_decision(
        handle: int,
        source_client_id: str,
        thread_id: str,
        request_id: JsonValue,
        decision_payload: JsonValue,
        timeout_sec: float,
        owner_clients: ipc_turn.OwnerClients,
        method: str,
        use_target_client: bool,
    ) -> JsonObject:
        _ = (handle, source_client_id, thread_id, timeout_sec, owner_clients)
        approval_calls.append((request_id, decision_payload, method, use_target_client))
        return {"ok": True}

    def approval_decision_method_for_request_kind(request_kind: str) -> str:
        return f"method:{request_kind}"

    def activate_thread_in_ui(thread: ThreadInfo) -> str:
        _ = thread
        events.append("activate")
        return activation_result

    def discover_owner_client_for_thread(handle: int, thread_id: str, timeout_sec: float) -> str | None:
        events.append(f"discover:{handle}:{thread_id}:{timeout_sec}")
        if pending_discoveries:
            return pending_discoveries.pop(0)
        return None

    def sleep(seconds: float) -> None:
        events.append(f"sleep:{seconds}")

    deps = ipc_turn.IpcTurnDeps(
        open_ipc_pipe=open_ipc_pipe,
        close_ipc_pipe=close_ipc_pipe,
        initialize_ipc_client=initialize_ipc_client,
        request_start_turn=request_start_turn,
        request_submit_user_input=request_submit_user_input,
        request_submit_approval_decision=request_submit_approval_decision,
        approval_decision_method_for_request_kind=approval_decision_method_for_request_kind,
        activate_thread_in_ui=activate_thread_in_ui,
        discover_owner_client_for_thread=discover_owner_client_for_thread,
        sleep=sleep,
    )
    return IpcTurnHarness(
        deps=deps,
        events=events,
        start_owner_snapshots=start_owner_snapshots,
        approval_calls=approval_calls,
    )


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


if __name__ == "__main__":
    _ = unittest.main()

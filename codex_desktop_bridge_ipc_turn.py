from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_bridge_state import JsonObject, JsonValue
from codex_desktop_bridge_ipc_start_turn import IPCNoClientFoundError
from codex_thread_models import ThreadInfo


OwnerClients = dict[str, str]
ActivateThreadInUi = Callable[[ThreadInfo], str]
ApprovalDecisionMethodForKind = Callable[[str], str]
CloseIpcPipe = Callable[[int], None]
DiscoverOwnerClient = Callable[[int, str, float], str | None]
InitializeIpcClient = Callable[[int, OwnerClients, float], str]
OpenIpcPipe = Callable[[], int]
RequestStartTurn = Callable[[int, str, ThreadInfo, str, float, OwnerClients], dict[str, str]]
RequestSubmitApprovalDecision = Callable[[int, str, str, JsonValue, JsonValue, float, OwnerClients, str, bool], JsonObject]
RequestSubmitUserInput = Callable[[int, str, str, str, JsonObject, float, OwnerClients], JsonObject]
Sleep = Callable[[float], None]


class IPCOwnerClientMissingError(RuntimeError):
    def __init__(self, *, allow_ui_recovery: bool, last_activation_error: str) -> None:
        self.allow_ui_recovery: bool = allow_ui_recovery
        self.last_activation_error: str = last_activation_error
        super().__init__(_ipc_owner_missing_detail(allow_ui_recovery, last_activation_error))


class IPCStartTurnNoResultError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("IPC start-turn exited without a result.")


class IpcThread(Protocol):
    @property
    def id(self) -> str: ...


@dataclass(frozen=True, slots=True)
class IpcTurnDeps:
    open_ipc_pipe: OpenIpcPipe
    close_ipc_pipe: CloseIpcPipe
    initialize_ipc_client: InitializeIpcClient
    request_start_turn: RequestStartTurn
    request_submit_user_input: RequestSubmitUserInput
    request_submit_approval_decision: RequestSubmitApprovalDecision
    approval_decision_method_for_request_kind: ApprovalDecisionMethodForKind
    activate_thread_in_ui: ActivateThreadInUi
    discover_owner_client_for_thread: DiscoverOwnerClient
    sleep: Sleep


def start_turn_via_ipc(
    thread: ThreadInfo,
    prompt: str,
    timeout_sec: float,
    *,
    allow_ui_recovery: bool,
    deps: IpcTurnDeps,
) -> dict[str, str]:
    handle = deps.open_ipc_pipe()
    owner_clients: OwnerClients = {}
    recovery_method = ""
    last_activation_error = ""
    max_attempts = 3 if allow_ui_recovery else 2
    retry_sleep_base = 0.75 if allow_ui_recovery else 0.35
    discover_timeout_sec = (
        max(2.0, min(timeout_sec, 6.0))
        if allow_ui_recovery
        else max(0.75, min(timeout_sec, 1.5))
    )
    try:
        source_client_id = deps.initialize_ipc_client(handle, owner_clients, min(timeout_sec, 3.0))
        for attempt in range(max_attempts):
            try:
                result = deps.request_start_turn(
                    handle,
                    source_client_id,
                    thread,
                    prompt,
                    timeout_sec,
                    owner_clients,
                )
                if recovery_method:
                    result["recovery_method"] = recovery_method
                return result
            except IPCNoClientFoundError:
                if attempt >= (max_attempts - 1):
                    raise IPCOwnerClientMissingError(
                        allow_ui_recovery=allow_ui_recovery,
                        last_activation_error=last_activation_error,
                    )
                if allow_ui_recovery:
                    recovery_method, last_activation_error = _try_activate_thread(thread, deps)
                deps.sleep(retry_sleep_base * (attempt + 1))
                discovered_owner = deps.discover_owner_client_for_thread(
                    handle,
                    thread.id,
                    discover_timeout_sec,
                )
                if discovered_owner:
                    owner_clients[thread.id] = discovered_owner
    finally:
        deps.close_ipc_pipe(handle)
    raise IPCStartTurnNoResultError()


def submit_user_input_via_ipc(
    thread: IpcThread,
    request_id: str,
    response_payload: JsonObject,
    timeout_sec: float,
    deps: IpcTurnDeps,
) -> JsonObject:
    handle = deps.open_ipc_pipe()
    owner_clients: OwnerClients = {}
    try:
        source_client_id = deps.initialize_ipc_client(handle, owner_clients, min(timeout_sec, 3.0))
        _prime_owner_client(handle, thread.id, timeout_sec, owner_clients, deps)
        result = deps.request_submit_user_input(
            handle,
            source_client_id,
            thread.id,
            request_id,
            response_payload,
            timeout_sec,
            owner_clients,
        )
        _ = result.setdefault("request_id", request_id)
        return result
    finally:
        deps.close_ipc_pipe(handle)


def submit_approval_decision_via_ipc(
    thread: IpcThread,
    request_id: JsonValue,
    decision_payload: JsonValue,
    request_kind: str,
    timeout_sec: float,
    *,
    use_target_client: bool,
    deps: IpcTurnDeps,
) -> JsonObject:
    method = deps.approval_decision_method_for_request_kind(request_kind)
    handle = deps.open_ipc_pipe()
    owner_clients: OwnerClients = {}
    try:
        source_client_id = deps.initialize_ipc_client(handle, owner_clients, min(timeout_sec, 3.0))
        _prime_owner_client(handle, thread.id, timeout_sec, owner_clients, deps)
        result = deps.request_submit_approval_decision(
            handle,
            source_client_id,
            thread.id,
            request_id,
            decision_payload,
            timeout_sec,
            owner_clients,
            method,
            use_target_client,
        )
        _ = result.setdefault("request_id", str(request_id))
        _ = result.setdefault("request_kind", request_kind)
        return result
    finally:
        deps.close_ipc_pipe(handle)


def _try_activate_thread(thread: ThreadInfo, deps: IpcTurnDeps) -> tuple[str, str]:
    try:
        return deps.activate_thread_in_ui(thread), ""
    except RuntimeError as exc:
        return "", str(exc)


def _prime_owner_client(
    handle: int,
    thread_id: str,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcTurnDeps,
) -> None:
    discovered_owner = deps.discover_owner_client_for_thread(
        handle,
        thread_id,
        max(0.75, min(timeout_sec, 2.5)),
    )
    if discovered_owner:
        owner_clients[thread_id] = discovered_owner


def _ipc_owner_missing_detail(allow_ui_recovery: bool, last_activation_error: str) -> str:
    if not allow_ui_recovery:
        return (
            "IPC owner client for the selected thread was not discovered in background mode. "
            "The target thread may still be loading. Open that thread once, wait a few seconds, "
            "or rerun with --ipc-recover-ui if you want an automatic UI recovery attempt."
        )
    detail = (
        "IPC owner client for the selected thread was not discovered even after re-activating "
        "the thread in the Codex UI. The app is likely still loading or lagging. Wait a few "
        "seconds, open the thread once, and retry."
    )
    if last_activation_error:
        detail += f" Last activation error: {last_activation_error}"
    return detail

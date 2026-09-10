from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import codex_desktop_bridge_ipc_session as ipc_session
import codex_desktop_bridge_ipc_start_turn as ipc_start_turn
import codex_desktop_bridge_ipc_submit as ipc_submit
from codex_bridge_state import JsonObject, JsonValue
from codex_thread_models import ThreadInfo

OwnerClients = dict[str, str]
ReadIpcMessage = Callable[[int, float], JsonObject]
ReadIpcResponse = Callable[[int, str, float, OwnerClients], JsonObject]
TimeNow = Callable[[], float]
UuidFactory = Callable[[], str]
WriteIpcMessage = Callable[[int, JsonObject], None]

IPCNoClientFoundError = ipc_start_turn.IPCNoClientFoundError
approval_decision_method_for_request_kind = ipc_submit.approval_decision_method_for_request_kind


@dataclass(frozen=True, slots=True)
class IpcRuntimeDeps:
    read_ipc_message: ReadIpcMessage
    write_ipc_message: WriteIpcMessage
    read_ipc_response: ReadIpcResponse
    uuid_factory: UuidFactory
    time_now: TimeNow
    pipe_peek_retry_sec: float


record_owner_client_from_ipc_message = ipc_session.record_owner_client_from_ipc_message


extract_thread_snapshot_from_ipc_message = ipc_session.extract_thread_snapshot_from_ipc_message


def read_ipc_response(
    handle: int,
    request_id: str,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcRuntimeDeps,
) -> JsonObject:
    return ipc_session.read_ipc_response(
        handle,
        request_id,
        timeout_sec,
        owner_clients,
        _make_ipc_session_deps(deps),
    )


def initialize_ipc_client(
    handle: int,
    owner_clients: OwnerClients,
    timeout_sec: float,
    deps: IpcRuntimeDeps,
) -> str:
    return ipc_session.initialize_ipc_client(
        handle,
        owner_clients,
        timeout_sec,
        _make_ipc_session_deps(deps),
    )


def discover_owner_client_for_thread(
    handle: int,
    thread_id: str,
    timeout_sec: float,
    deps: IpcRuntimeDeps,
) -> str | None:
    return ipc_session.discover_owner_client_for_thread(
        handle,
        thread_id,
        timeout_sec,
        _make_ipc_session_deps(deps),
    )


def request_start_turn_via_ipc(
    handle: int,
    source_client_id: str,
    thread: ThreadInfo,
    prompt: str,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcRuntimeDeps,
) -> dict[str, str]:
    return ipc_start_turn.request_start_turn_via_ipc(
        handle=handle,
        source_client_id=source_client_id,
        thread=thread,
        prompt=prompt,
        timeout_sec=timeout_sec,
        owner_clients=owner_clients,
        deps=_make_start_turn_deps(deps),
    )


def request_submit_user_input_via_ipc(
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: str,
    response_payload: JsonObject,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcRuntimeDeps,
) -> JsonObject:
    return ipc_submit.request_submit_user_input_via_ipc(
        handle=handle,
        source_client_id=source_client_id,
        thread_id=thread_id,
        request_id=request_id,
        response_payload=response_payload,
        timeout_sec=timeout_sec,
        owner_clients=owner_clients,
        deps=_make_submit_deps(deps),
    )


def request_submit_approval_decision_via_ipc(
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: JsonValue,
    decision_payload: JsonValue,
    timeout_sec: float,
    owner_clients: OwnerClients,
    method: str,
    use_target_client: bool,
    deps: IpcRuntimeDeps,
) -> JsonObject:
    return ipc_submit.request_submit_approval_decision_via_ipc(
        handle=handle,
        source_client_id=source_client_id,
        thread_id=thread_id,
        request_id=request_id,
        decision_payload=decision_payload,
        timeout_sec=timeout_sec,
        owner_clients=owner_clients,
        method=method,
        deps=_make_submit_deps(deps),
        use_target_client=use_target_client,
    )


def _make_ipc_session_deps(deps: IpcRuntimeDeps) -> ipc_session.IpcSessionDeps:
    return ipc_session.IpcSessionDeps(
        read_ipc_message=deps.read_ipc_message,
        write_ipc_message=deps.write_ipc_message,
        read_ipc_response=deps.read_ipc_response,
        uuid_factory=deps.uuid_factory,
        time_now=deps.time_now,
        pipe_peek_retry_sec=deps.pipe_peek_retry_sec,
    )


def _make_start_turn_deps(deps: IpcRuntimeDeps) -> ipc_start_turn.IpcStartTurnDeps:
    return ipc_start_turn.IpcStartTurnDeps(
        write_ipc_message=deps.write_ipc_message,
        read_ipc_response=deps.read_ipc_response,
    )


def _make_submit_deps(deps: IpcRuntimeDeps) -> ipc_submit.IpcSubmitDeps:
    return ipc_submit.IpcSubmitDeps(
        write_ipc_message=deps.write_ipc_message,
        read_ipc_response=deps.read_ipc_response,
    )

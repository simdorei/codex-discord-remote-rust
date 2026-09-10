from __future__ import annotations

import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Final, TypeAlias

from codex_desktop_bridge_ipc_start_turn import IPCNoClientFoundError
from codex_desktop_bridge_ipc_submit_requests import (
    approval_decision_method_for_request_kind as approval_decision_method_for_request_kind,
    build_submit_approval_decision_request,
    build_submit_user_input_request,
)

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
OwnerClients: TypeAlias = dict[str, str]

WriteIpcMessage = Callable[[int, JsonObject], None]
ReadIpcResponse = Callable[[int, str, float, OwnerClients], JsonObject]

__all__: Final = [
    "IPCNoClientFoundError",
    "IPCSubmitResponseError",
    "IpcSubmitDeps",
    "JsonObject",
    "JsonValue",
    "approval_decision_method_for_request_kind",
    "request_submit_approval_decision_via_ipc",
    "request_submit_user_input_via_ipc",
]


class IPCSubmitResponseError(RuntimeError):
    def __init__(self, message: str) -> None:
        self.message: str = message
        super().__init__(message)


@dataclass(frozen=True, slots=True)
class IpcSubmitDeps:
    write_ipc_message: WriteIpcMessage
    read_ipc_response: ReadIpcResponse


def request_submit_user_input_via_ipc(
    *,
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: str,
    response_payload: JsonObject,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcSubmitDeps,
) -> JsonObject:
    owner_client_id = owner_clients.get(thread_id)
    ipc_request_id = str(uuid.uuid4())
    request = build_submit_user_input_request(
        ipc_request_id=ipc_request_id,
        source_client_id=source_client_id,
        thread_id=thread_id,
        request_id=request_id,
        response_payload=response_payload,
        owner_client_id=owner_client_id,
    )
    deps.write_ipc_message(handle, request)
    response = deps.read_ipc_response(handle, ipc_request_id, timeout_sec, owner_clients)
    return _parse_submit_response(
        response,
        failure_prefix="IPC submit-user-input failed",
        invalid_payload_message="IPC submit-user-input returned an invalid payload.",
        thread_id=thread_id,
        owner_client_id=owner_client_id,
        owner_clients=owner_clients,
    )


def request_submit_approval_decision_via_ipc(
    *,
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: JsonValue,
    decision_payload: JsonValue,
    timeout_sec: float,
    owner_clients: OwnerClients,
    method: str,
    deps: IpcSubmitDeps,
    use_target_client: bool = True,
) -> JsonObject:
    owner_client_id = owner_clients.get(thread_id)
    ipc_request_id = str(uuid.uuid4())
    request = build_submit_approval_decision_request(
        ipc_request_id=ipc_request_id,
        source_client_id=source_client_id,
        thread_id=thread_id,
        request_id=request_id,
        decision_payload=decision_payload,
        owner_client_id=owner_client_id,
        method=method,
        use_target_client=use_target_client,
    )
    deps.write_ipc_message(handle, request)
    response = deps.read_ipc_response(handle, ipc_request_id, timeout_sec, owner_clients)
    return _parse_submit_response(
        response,
        failure_prefix="IPC approval decision failed",
        invalid_payload_message="IPC approval decision returned an invalid payload.",
        thread_id=thread_id,
        owner_client_id=owner_client_id,
        owner_clients=owner_clients,
    )


def _parse_submit_response(
    response: JsonObject,
    *,
    failure_prefix: str,
    invalid_payload_message: str,
    thread_id: str,
    owner_client_id: str | None,
    owner_clients: OwnerClients,
) -> JsonObject:
    if response.get("resultType") != "success":
        error = _clean_string(response.get("error")) or "unknown error"
        if "no-client-found" in error:
            raise IPCNoClientFoundError(error)
        raise IPCSubmitResponseError(f"{failure_prefix}: {error}")

    payload = response.get("result") or {}
    if not isinstance(payload, dict):
        raise RuntimeError(invalid_payload_message)

    _ = payload.setdefault(
        "owner_client_id",
        _clean_string(response.get("handledByClientId"))
        or owner_clients.get(thread_id)
        or owner_client_id
        or "",
    )
    return payload


def _clean_string(value: JsonValue | str | None) -> str:
    return value.strip() if isinstance(value, str) else ""

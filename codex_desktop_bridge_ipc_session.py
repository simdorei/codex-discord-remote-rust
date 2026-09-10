from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import cast

from codex_bridge_state import JsonObject, JsonValue


OwnerClients = dict[str, str]
ReadIpcMessage = Callable[[int, float], JsonObject]
ReadIpcResponse = Callable[[int, str, float, OwnerClients], JsonObject]
TimeNow = Callable[[], float]
UuidFactory = Callable[[], str]
WriteIpcMessage = Callable[[int, JsonObject], None]


class IPCInitializeResponseError(RuntimeError):
    def __init__(self, message: str) -> None:
        self.message: str = message
        super().__init__(message)


class IPCInitializeInvalidPayloadError(RuntimeError):
    pass


class IPCInitializeMissingClientIdError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class IpcSessionDeps:
    read_ipc_message: ReadIpcMessage
    write_ipc_message: WriteIpcMessage
    read_ipc_response: ReadIpcResponse
    uuid_factory: UuidFactory
    time_now: TimeNow
    pipe_peek_retry_sec: float


def record_owner_client_from_ipc_message(message: JsonObject, owner_clients: OwnerClients) -> None:
    if message.get("type") != "broadcast" or message.get("method") != "thread-stream-state-changed":
        return
    params = _object_value(message.get("params"))
    if params is None:
        return
    conversation_id = str(params.get("conversationId") or "").strip()
    source_client_id = str(message.get("sourceClientId") or "").strip()
    if conversation_id and source_client_id:
        owner_clients[conversation_id] = source_client_id


def extract_thread_snapshot_from_ipc_message(
    message: JsonObject,
    thread_id: str,
) -> tuple[JsonObject, str] | None:
    if message.get("type") != "broadcast" or message.get("method") != "thread-stream-state-changed":
        return None
    params = _object_value(message.get("params"))
    if params is None or str(params.get("conversationId") or "").strip() != thread_id:
        return None
    change = _object_value(params.get("change"))
    if change is None or str(change.get("type") or "").strip() != "snapshot":
        return None
    conversation_state = _object_value(change.get("conversationState"))
    if conversation_state is None:
        return None
    owner_client_id = str(message.get("sourceClientId") or "").strip()
    return conversation_state, owner_client_id


def read_ipc_response(
    handle: int,
    request_id: str,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcSessionDeps,
) -> JsonObject:
    deadline = deps.time_now() + timeout_sec
    while deps.time_now() < deadline:
        message = deps.read_ipc_message(handle, max(deps.pipe_peek_retry_sec, deadline - deps.time_now()))
        _decline_unhandled_client_discovery(handle, message, deps)
        record_owner_client_from_ipc_message(message, owner_clients)
        if message.get("type") == "response" and message.get("requestId") == request_id:
            return message
    raise TimeoutError(f"Timed out waiting for IPC response to request {request_id}.")


def initialize_ipc_client(
    handle: int,
    owner_clients: OwnerClients,
    timeout_sec: float,
    deps: IpcSessionDeps,
) -> str:
    request_id = deps.uuid_factory()
    deps.write_ipc_message(
        handle,
        {
            "type": "request",
            "requestId": request_id,
            "sourceClientId": "initializing-client",
            "version": 0,
            "method": "initialize",
            "params": {"clientType": "codex-desktop-bridge"},
        },
    )
    response = deps.read_ipc_response(handle, request_id, timeout_sec, owner_clients)
    if response.get("resultType") != "success":
        raise IPCInitializeResponseError(f"IPC initialize failed: {response.get('error') or 'unknown error'}")
    result = _object_value(response.get("result"))
    if result is None:
        raise IPCInitializeInvalidPayloadError("IPC initialize returned an invalid payload.")
    client_id = str(result.get("clientId") or "").strip()
    if not client_id:
        raise IPCInitializeMissingClientIdError("IPC initialize did not return a clientId.")
    return client_id


def discover_owner_client_for_thread(
    handle: int,
    thread_id: str,
    timeout_sec: float,
    deps: IpcSessionDeps,
) -> str | None:
    owner_clients: OwnerClients = {}
    deadline = deps.time_now() + timeout_sec
    while deps.time_now() < deadline:
        if thread_id in owner_clients:
            return owner_clients[thread_id]
        try:
            message = deps.read_ipc_message(handle, max(deps.pipe_peek_retry_sec, deadline - deps.time_now()))
        except TimeoutError:
            return owner_clients.get(thread_id)
        _decline_unhandled_client_discovery(handle, message, deps)
        record_owner_client_from_ipc_message(message, owner_clients)
        if thread_id in owner_clients:
            return owner_clients[thread_id]
    return None


def _decline_unhandled_client_discovery(
    handle: int,
    message: JsonObject,
    deps: IpcSessionDeps,
) -> None:
    if message.get("type") != "client-discovery-request":
        return
    request_id = str(message.get("requestId") or "").strip()
    if not request_id:
        return
    deps.write_ipc_message(
        handle,
        {
            "type": "client-discovery-response",
            "requestId": request_id,
            "response": {"canHandle": False},
        },
    )


def _object_value(value: JsonValue | None) -> JsonObject | None:
    return cast(JsonObject, value) if isinstance(value, dict) else None

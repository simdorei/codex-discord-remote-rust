from __future__ import annotations

import uuid
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

from codex_desktop_bridge_ipc_start_turn_requests import build_start_turn_request

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
OwnerClients: TypeAlias = dict[str, str]

WriteIpcMessage = Callable[[int, JsonObject], None]
ReadIpcResponse = Callable[[int, str, float, OwnerClients], JsonObject]


class IpcThread(Protocol):
    @property
    def id(self) -> str: ...


class IPCNoClientFoundError(RuntimeError):
    pass


class IPCStartTurnResponseError(RuntimeError):
    def __init__(self, message: str) -> None:
        self.message: str = message
        super().__init__(message)


class IPCStartTurnInvalidPayloadError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("IPC start-turn returned an invalid payload.")


@dataclass(frozen=True, slots=True)
class IpcStartTurnDeps:
    write_ipc_message: WriteIpcMessage
    read_ipc_response: ReadIpcResponse


def request_start_turn_via_ipc(
    *,
    handle: int,
    source_client_id: str,
    thread: IpcThread,
    prompt: str,
    timeout_sec: float,
    owner_clients: OwnerClients,
    deps: IpcStartTurnDeps,
) -> dict[str, str]:
    owner_client_id = owner_clients.get(thread.id)
    request_id = str(uuid.uuid4())
    request = build_start_turn_request(
        request_id=request_id,
        source_client_id=source_client_id,
        thread_id=thread.id,
        prompt=prompt,
        owner_client_id=owner_client_id,
    )
    deps.write_ipc_message(handle, request)
    response = deps.read_ipc_response(
        handle,
        request_id,
        timeout_sec,
        owner_clients,
    )
    return _parse_start_turn_response(
        response,
        thread_id=thread.id,
        owner_client_id=owner_client_id,
        owner_clients=owner_clients,
    )

def _parse_start_turn_response(
    response: JsonObject,
    *,
    thread_id: str,
    owner_client_id: str | None,
    owner_clients: OwnerClients,
) -> dict[str, str]:
    if response.get("resultType") != "success":
        error = _clean_string(response.get("error")) or "unknown error"
        if "no-client-found" in error:
            raise IPCNoClientFoundError(error)
        raise IPCStartTurnResponseError(f"IPC start-turn failed: {error}")

    payload = response.get("result")
    if not isinstance(payload, dict):
        raise IPCStartTurnInvalidPayloadError()

    nested_result = payload.get("result")
    turn_id = ""
    if isinstance(nested_result, dict):
        turn = nested_result.get("turn")
        if isinstance(turn, dict):
            turn_id = _clean_string(turn.get("id"))

    handled_by_client_id = (
        _clean_string(response.get("handledByClientId"))
        or owner_clients.get(thread_id)
        or owner_client_id
        or ""
    )
    return {
        "owner_client_id": handled_by_client_id,
        "turn_id": turn_id,
    }


def _clean_string(value: JsonValue | str | None) -> str:
    return value.strip() if isinstance(value, str) else ""

from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

from codex_desktop_bridge_pending_requests import (
    JsonObject,
    extract_pending_approval_request,
    extract_pending_user_input_request,
)


OwnerClients: TypeAlias = dict[str, str]
ThreadSnapshot: TypeAlias = tuple[JsonObject, str]
CloseIpcPipe = Callable[[int], None]
OpenIpcPipe = Callable[[], int]
ReadIpcMessage = Callable[[int, float], JsonObject]
RecordOwnerClient = Callable[[JsonObject, OwnerClients], None]
ExtractThreadSnapshot = Callable[[JsonObject, str], ThreadSnapshot | None]
CacheApprovalRequest = Callable[[JsonObject], None]
TimeNow = Callable[[], float]


class PendingThread(Protocol):
    @property
    def id(self) -> str: ...


class InitializeIpcClient(Protocol):
    def __call__(self, handle: int, owner_clients: OwnerClients, timeout_sec: float = 3.0) -> str: ...


@dataclass(frozen=True, slots=True)
class IpcPendingDeps:
    read_ipc_message: ReadIpcMessage
    record_owner_client_from_ipc_message: RecordOwnerClient
    extract_thread_snapshot_from_ipc_message: ExtractThreadSnapshot
    cache_live_approval_request: CacheApprovalRequest
    time_now: TimeNow = time.time


@dataclass(frozen=True, slots=True)
class IpcPendingRuntimeDeps:
    open_ipc_pipe: OpenIpcPipe
    close_ipc_pipe: CloseIpcPipe
    initialize_ipc_client: InitializeIpcClient
    pending_deps: IpcPendingDeps


def get_pending_approval_request_via_ipc(
    *,
    handle: int,
    thread: PendingThread,
    timeout_sec: float,
    owner_clients: OwnerClients,
    pipe_peek_retry_sec: float,
    deps: IpcPendingDeps,
) -> JsonObject | None:
    return _poll_pending_request_via_ipc(
        handle=handle,
        thread=thread,
        timeout_sec=timeout_sec,
        owner_clients=owner_clients,
        pipe_peek_retry_sec=pipe_peek_retry_sec,
        deps=deps,
        extract_pending_request=extract_pending_approval_request,
        stop_on_snapshot_without_request=False,
        cache_pending_request=deps.cache_live_approval_request,
    )


def get_pending_approval_request_via_ipc_pipe(
    thread: PendingThread,
    *,
    timeout_sec: float,
    pipe_peek_retry_sec: float,
    deps: IpcPendingRuntimeDeps,
) -> JsonObject | None:
    handle = deps.open_ipc_pipe()
    owner_clients: OwnerClients = {}
    try:
        _ = deps.initialize_ipc_client(handle, owner_clients, timeout_sec=min(timeout_sec, 3.0))
        return get_pending_approval_request_via_ipc(
            handle=handle,
            thread=thread,
            timeout_sec=timeout_sec,
            owner_clients=owner_clients,
            pipe_peek_retry_sec=pipe_peek_retry_sec,
            deps=deps.pending_deps,
        )
    finally:
        deps.close_ipc_pipe(handle)


def get_pending_user_input_request_via_ipc(
    *,
    handle: int,
    thread: PendingThread,
    timeout_sec: float,
    owner_clients: OwnerClients,
    pipe_peek_retry_sec: float,
    deps: IpcPendingDeps,
) -> JsonObject | None:
    return _poll_pending_request_via_ipc(
        handle=handle,
        thread=thread,
        timeout_sec=timeout_sec,
        owner_clients=owner_clients,
        pipe_peek_retry_sec=pipe_peek_retry_sec,
        deps=deps,
        extract_pending_request=extract_pending_user_input_request,
        stop_on_snapshot_without_request=True,
        cache_pending_request=None,
    )


def get_pending_user_input_request_via_ipc_pipe(
    thread: PendingThread,
    *,
    timeout_sec: float,
    pipe_peek_retry_sec: float,
    deps: IpcPendingRuntimeDeps,
) -> JsonObject | None:
    handle = deps.open_ipc_pipe()
    owner_clients: OwnerClients = {}
    try:
        _ = deps.initialize_ipc_client(handle, owner_clients, timeout_sec=min(timeout_sec, 3.0))
        return get_pending_user_input_request_via_ipc(
            handle=handle,
            thread=thread,
            timeout_sec=timeout_sec,
            owner_clients=owner_clients,
            pipe_peek_retry_sec=pipe_peek_retry_sec,
            deps=deps.pending_deps,
        )
    finally:
        deps.close_ipc_pipe(handle)


def _poll_pending_request_via_ipc(
    *,
    handle: int,
    thread: PendingThread,
    timeout_sec: float,
    owner_clients: OwnerClients,
    pipe_peek_retry_sec: float,
    deps: IpcPendingDeps,
    extract_pending_request: Callable[[JsonObject, str], JsonObject | None],
    stop_on_snapshot_without_request: bool,
    cache_pending_request: CacheApprovalRequest | None,
) -> JsonObject | None:
    deadline = deps.time_now() + max(timeout_sec, 0.0)
    while deps.time_now() < deadline:
        try:
            message = deps.read_ipc_message(
                handle,
                max(pipe_peek_retry_sec, deadline - deps.time_now()),
            )
        except TimeoutError:
            break
        deps.record_owner_client_from_ipc_message(message, owner_clients)
        snapshot = deps.extract_thread_snapshot_from_ipc_message(message, thread.id)
        if snapshot is None:
            continue
        conversation_state, owner_client_id = snapshot
        if owner_client_id:
            owner_clients[thread.id] = owner_client_id
        pending_request = extract_pending_request(conversation_state, thread.id)
        if pending_request is None:
            if stop_on_snapshot_without_request:
                return None
            continue
        _ = pending_request.setdefault(
            "owner_client_id",
            owner_clients.get(thread.id) or owner_client_id or "",
        )
        if cache_pending_request is not None:
            cache_pending_request(pending_request)
        return pending_request
    return None

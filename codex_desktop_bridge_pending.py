from __future__ import annotations

from collections.abc import Callable
from typing import TypeAlias

import codex_desktop_bridge_pending_ipc_pipe as pending_ipc_pipe
from codex_desktop_bridge_pending_requests import (
    JsonObject,
    JsonScalar,
    JsonValue,
    clean_request_string,
    extract_pending_approval_request,
    extract_pending_user_input_request,
)

OwnerClients: TypeAlias = pending_ipc_pipe.OwnerClients
ThreadSnapshot: TypeAlias = pending_ipc_pipe.ThreadSnapshot
CloseIpcPipe: TypeAlias = pending_ipc_pipe.CloseIpcPipe
OpenIpcPipe: TypeAlias = pending_ipc_pipe.OpenIpcPipe
ReadIpcMessage: TypeAlias = pending_ipc_pipe.ReadIpcMessage
RecordOwnerClient: TypeAlias = pending_ipc_pipe.RecordOwnerClient
ExtractThreadSnapshot: TypeAlias = pending_ipc_pipe.ExtractThreadSnapshot
CacheApprovalRequest: TypeAlias = pending_ipc_pipe.CacheApprovalRequest
GetPendingApproval = Callable[["PendingThread", float], JsonObject | None]
CollapseListText = Callable[[str, int], str]
TimeNow: TypeAlias = pending_ipc_pipe.TimeNow
PendingThread: TypeAlias = pending_ipc_pipe.PendingThread
InitializeIpcClient: TypeAlias = pending_ipc_pipe.InitializeIpcClient
IpcPendingDeps: TypeAlias = pending_ipc_pipe.IpcPendingDeps
IpcPendingRuntimeDeps: TypeAlias = pending_ipc_pipe.IpcPendingRuntimeDeps
get_pending_approval_request_via_ipc = pending_ipc_pipe.get_pending_approval_request_via_ipc
get_pending_approval_request_via_ipc_pipe = pending_ipc_pipe.get_pending_approval_request_via_ipc_pipe
get_pending_user_input_request_via_ipc = pending_ipc_pipe.get_pending_user_input_request_via_ipc
get_pending_user_input_request_via_ipc_pipe = pending_ipc_pipe.get_pending_user_input_request_via_ipc_pipe

__all__ = [
    "CacheApprovalRequest",
    "CloseIpcPipe",
    "CollapseListText",
    "ExtractThreadSnapshot",
    "InitializeIpcClient",
    "GetPendingApproval",
    "IpcPendingDeps",
    "IpcPendingRuntimeDeps",
    "JsonObject",
    "JsonScalar",
    "JsonValue",
    "OpenIpcPipe",
    "OwnerClients",
    "PendingThread",
    "ReadIpcMessage",
    "RecordOwnerClient",
    "ThreadSnapshot",
    "TimeNow",
    "extract_pending_approval_request",
    "extract_pending_user_input_request",
    "get_live_pending_approval_display_lines",
    "get_pending_approval_request_via_ipc",
    "get_pending_approval_request_via_ipc_pipe",
    "get_pending_user_input_request_via_ipc",
    "get_pending_user_input_request_via_ipc_pipe",
]

def get_live_pending_approval_display_lines(
    thread: PendingThread,
    *,
    timeout_sec: float,
    reason_limit: int,
    get_pending_approval_request: GetPendingApproval,
    collapse_list_text: CollapseListText,
) -> tuple[str | None, list[str]]:
    pending_request = get_pending_approval_request(thread, timeout_sec)
    if pending_request is None:
        return None, []

    lines: list[str] = []
    request_kind = clean_request_string(pending_request.get("request_kind"))
    if request_kind:
        lines.append(f"kind: {request_kind}")

    reason = collapse_list_text(clean_request_string(pending_request.get("reason")), reason_limit)
    if reason:
        lines.append(reason)

    lines.extend(
        [
            "1. yes",
            "2. yes + do not ask again in this session",
            "3. reject + submit reason",
        ]
    )

    if not lines:
        lines.append("Approval request is pending.")
    return "waiting-approval", lines

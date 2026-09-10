from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import typed_close_ipc_pipe as _close_ipc_pipe, typed_discover_owner_client_for_thread as _discover_owner_client_for_thread, typed_extract_thread_snapshot_from_ipc_message as _extract_thread_snapshot_from_ipc_message, typed_initialize_ipc_client as _initialize_ipc_client, typed_make_ipc_runtime_deps as _make_ipc_runtime_deps, typed_make_pending_reply_deps as _make_pending_reply_deps, typed_open_codex_ipc_pipe as _open_codex_ipc_pipe, typed_read_ipc_message as _read_ipc_message, typed_record_owner_client_from_ipc_message as _record_owner_client_from_ipc_message, typed_request_start_turn_via_ipc as _request_start_turn_via_ipc, activate_thread_for_ipc, cache_live_approval_request, collapse_list_text

def _request_submit_user_input_via_ipc(
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: str,
    response_payload: JsonObject,
    timeout_sec: float,
    owner_clients: dict[str, str],
) -> JsonObject:
    return ipc_runtime.request_submit_user_input_via_ipc(
        handle, source_client_id, thread_id, request_id, response_payload, timeout_sec, owner_clients, _make_ipc_runtime_deps()
    )

def start_turn_via_ipc(
    thread: ThreadInfo,
    prompt: str,
    timeout_sec: float = 4.0,
    *,
    allow_ui_recovery: bool = False,
) -> dict[str, str]:
    return ipc_turn.start_turn_via_ipc(
        thread,
        prompt,
        timeout_sec,
        allow_ui_recovery=allow_ui_recovery,
        deps=_make_ipc_turn_deps(),
    )

def submit_user_input_via_ipc(
    thread: ipc_turn.IpcThread,
    request_id: str,
    response_payload: JsonObject,
    timeout_sec: float = 6.0,
) -> JsonObject:
    return ipc_turn.submit_user_input_via_ipc(
        thread,
        request_id,
        response_payload,
        timeout_sec,
        _make_ipc_turn_deps(),
    )

def _request_submit_approval_decision_via_ipc(
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: JsonValue,
    decision_payload: JsonValue,
    timeout_sec: float,
    owner_clients: dict[str, str],
    *,
    method: str,
    use_target_client: bool = True,
) -> JsonObject:
    return ipc_runtime.request_submit_approval_decision_via_ipc(
        handle,
        source_client_id,
        thread_id,
        request_id,
        decision_payload,
        timeout_sec,
        owner_clients,
        method,
        use_target_client,
        _make_ipc_runtime_deps(),
    )

def submit_approval_decision_via_ipc(
    thread: ipc_turn.IpcThread,
    request_id: JsonValue,
    decision_payload: JsonValue,
    request_kind: str,
    timeout_sec: float = 6.0,
    *,
    use_target_client: bool = True,
) -> JsonObject:
    return ipc_turn.submit_approval_decision_via_ipc(
        thread,
        request_id,
        decision_payload,
        request_kind,
        timeout_sec,
        use_target_client=use_target_client,
        deps=_make_ipc_turn_deps(),
    )

def _make_ipc_turn_deps() -> ipc_turn.IpcTurnDeps:
    return ipc_turn.IpcTurnDeps(
        open_ipc_pipe=_open_codex_ipc_pipe,
        close_ipc_pipe=_close_ipc_pipe,
        initialize_ipc_client=_initialize_ipc_client,
        request_start_turn=_request_start_turn_via_ipc,
        request_submit_user_input=_request_submit_user_input_via_ipc,
        request_submit_approval_decision=_request_submit_approval_decision_for_ipc_turn,
        approval_decision_method_for_request_kind=ipc_runtime.approval_decision_method_for_request_kind,
        activate_thread_in_ui=activate_thread_for_ipc,
        discover_owner_client_for_thread=_discover_owner_client_for_thread,
        sleep=time.sleep,
    )

def _request_submit_approval_decision_for_ipc_turn(
    handle: int,
    source_client_id: str,
    thread_id: str,
    request_id: JsonValue,
    decision_payload: JsonValue,
    timeout_sec: float,
    owner_clients: dict[str, str],
    method: str,
    use_target_client: bool,
) -> JsonObject:
    return _request_submit_approval_decision_via_ipc(
        handle, source_client_id, thread_id, request_id, decision_payload, timeout_sec, owner_clients, method=method, use_target_client=use_target_client
    )

_extract_pending_approval_request = bridge_pending.extract_pending_approval_request

def get_pending_approval_request_via_ipc(
    thread: bridge_pending.PendingThread,
    timeout_sec: float = 6.0,
) -> JsonObject | None:
    return bridge_pending.get_pending_approval_request_via_ipc_pipe(
        thread,
        timeout_sec=timeout_sec,
        pipe_peek_retry_sec=PIPE_PEEK_RETRY_SEC,
        deps=_make_pending_ipc_runtime_deps(),
    )

def get_live_pending_approval_display_lines(
    thread: ThreadInfo,
    *,
    timeout_sec: float = 1.0,
    reason_limit: int = 160,
) -> tuple[str | None, list[str]]:
    return bridge_pending.get_live_pending_approval_display_lines(
        thread,
        timeout_sec=timeout_sec,
        reason_limit=reason_limit,
        get_pending_approval_request=get_pending_approval_request_via_ipc,
        collapse_list_text=lambda text, limit: collapse_list_text(text, limit=limit),
    )

_extract_pending_user_input_request = bridge_pending.extract_pending_user_input_request

def get_pending_user_input_request_via_ipc(
    thread: bridge_pending.PendingThread,
    timeout_sec: float = 6.0,
) -> JsonObject | None:
    return bridge_pending.get_pending_user_input_request_via_ipc_pipe(
        thread,
        timeout_sec=timeout_sec,
        pipe_peek_retry_sec=PIPE_PEEK_RETRY_SEC,
        deps=_make_pending_ipc_runtime_deps(),
    )

def _make_pending_ipc_deps() -> bridge_pending.IpcPendingDeps:
    return bridge_pending.IpcPendingDeps(
        read_ipc_message=_read_ipc_message,
        record_owner_client_from_ipc_message=_record_owner_client_from_ipc_message,
        extract_thread_snapshot_from_ipc_message=_extract_thread_snapshot_from_ipc_message,
        cache_live_approval_request=cache_live_approval_request,
        time_now=time.time,
    )

def _make_pending_ipc_runtime_deps() -> bridge_pending.IpcPendingRuntimeDeps:
    return bridge_pending.IpcPendingRuntimeDeps(
        open_ipc_pipe=_open_codex_ipc_pipe,
        close_ipc_pipe=_close_ipc_pipe,
        initialize_ipc_client=_initialize_ipc_client,
        pending_deps=_make_pending_ipc_deps(),
    )

build_reply_input_response_payload = reply_payload.build_reply_input_response_payload

build_approval_decision_payload = reply_payload.build_approval_decision_payload

def reply_to_pending_user_input(
    thread: bridge_reply.ReplyThread,
    answer_text: str,
    timeout_sec: float = 6.0,
) -> bridge_reply.ReplyResult:
    return bridge_reply.reply_to_pending_user_input(
        thread,
        answer_text,
        timeout_sec,
        _make_pending_reply_deps(),
    )

def reply_to_pending_approval(
    thread: bridge_reply.ReplyThread,
    answer_text: str,
    timeout_sec: float = 6.0,
) -> bridge_reply.ReplyResult:
    return bridge_reply.reply_to_pending_approval(
        thread,
        answer_text,
        timeout_sec,
        _make_pending_reply_deps(),
    )

__all__ = ('_extract_pending_approval_request', '_extract_pending_user_input_request', '_make_ipc_turn_deps', '_make_pending_ipc_deps', '_make_pending_ipc_runtime_deps', '_request_submit_approval_decision_for_ipc_turn', '_request_submit_approval_decision_via_ipc', '_request_submit_user_input_via_ipc', 'annotations', 'build_approval_decision_payload', 'build_reply_input_response_payload', 'get_live_pending_approval_display_lines', 'get_pending_approval_request_via_ipc', 'get_pending_user_input_request_via_ipc', 'reply_to_pending_approval', 'reply_to_pending_user_input', 'start_turn_via_ipc', 'submit_approval_decision_via_ipc', 'submit_user_input_via_ipc')

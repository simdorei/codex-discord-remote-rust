from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import (
        load_recent_threads,
    )


extract_message_text = interactive_session.extract_message_text

parse_function_call_arguments = interactive_session.parse_function_call_arguments

build_interactive_notice_from_function_call = interactive_session.build_interactive_notice_from_function_call

classify_interactive_function_call = interactive_session.classify_interactive_function_call

get_pending_interactive_function_call_from_session = interactive_session.get_pending_interactive_function_call_from_session

get_pending_interactive_state_from_session = interactive_session.get_pending_interactive_state_from_session

get_pending_permission_approval_from_session = interactive_session.get_pending_permission_approval_from_session

get_pending_interactive_display_lines = interactive_session.get_pending_interactive_display_lines

summarize_interactive_lines = interactive_session.summarize_interactive_lines

get_pending_interactive_summary = interactive_session.get_pending_interactive_summary

get_last_user_and_assistant_messages = interactive_session.get_last_user_and_assistant_messages

get_thread_context_usage = thread_context.get_thread_context_usage

get_thread_collaboration_mode = thread_context.get_thread_collaboration_mode

get_thread_service_tier = thread_context.get_thread_service_tier

format_thread_model_display = thread_context.format_thread_model_display

describe_thread_context_usage = thread_context.describe_thread_context_usage

get_high_context_threads = thread_context.get_high_context_threads

should_recommend_archive = thread_context.should_recommend_archive

get_orphan_task_started_grace_seconds = thread_context.get_orphan_task_started_grace_seconds

get_stale_busy_session_seconds = thread_context.get_stale_busy_session_seconds


def _busy_state_ensure_thread_loaded(
    client: busy_state.SidecarClient,
    thread_id: str,
) -> busy_state.JsonObject:
    if isinstance(client, CodexAppServerSidecar):
        return sidecar_thread.ensure_thread_loaded_via_sidecar(client, thread_id)
    response = client.read_thread(thread_id, include_turns=False)
    thread_payload = response.get("thread")
    if isinstance(thread_payload, dict):
        return thread_payload
    return {}


def _make_busy_state_deps() -> busy_state.BusyStateDeps:
    return busy_state.BusyStateDeps(
        iter_session_events=iter_session_events,
        time_now=time.time,
        get_orphan_task_started_grace_seconds=get_orphan_task_started_grace_seconds,
        get_stale_busy_session_seconds=get_stale_busy_session_seconds,
        get_pending_interactive_state_from_session=get_pending_interactive_state_from_session,
        load_recent_threads=load_recent_threads,
        make_sidecar=CodexAppServerSidecar,
        get_sidecar_thread_status_type=sidecar_thread.get_sidecar_thread_status_type,
        ensure_thread_loaded_via_sidecar=_busy_state_ensure_thread_loaded,
    )


session_file_age_seconds = busy_state.session_file_age_seconds


def is_thread_busy(session_path: Path) -> bool:
    return busy_state.is_thread_busy(session_path, deps=_make_busy_state_deps())


def get_busy_threads(limit: int = 50) -> list[ThreadInfo]:
    return busy_state.get_busy_threads(limit=limit, deps=_make_busy_state_deps())


classify_thread_status = busy_state.classify_thread_status


def get_thread_busy_state(
    thread: busy_state.BusyThread,
    *,
    client: busy_state.SidecarClient | None = None,
    allow_resume: bool = False,
) -> str:
    return busy_state.get_thread_busy_state(
        thread,
        deps=_make_busy_state_deps(),
        client=client,
        allow_resume=allow_resume,
    )


describe_thread_busy_state = busy_state.describe_thread_busy_state

read_new_session_events = session_tail.read_new_session_events

_open_codex_ipc_pipe = ipc_pipe.open_codex_ipc_pipe

_read_ipc_message = ipc_pipe.read_ipc_message

_write_ipc_message = ipc_pipe.write_ipc_message

_close_ipc_pipe = ipc_pipe.close_ipc_pipe

_record_owner_client_from_ipc_message = ipc_runtime.record_owner_client_from_ipc_message

_extract_thread_snapshot_from_ipc_message = ipc_runtime.extract_thread_snapshot_from_ipc_message


def _read_ipc_response(
    handle: int,
    request_id: str,
    timeout_sec: float,
    owner_clients: dict[str, str],
) -> JsonObject:
    return ipc_runtime.read_ipc_response(
        handle,
        request_id,
        timeout_sec,
        owner_clients,
        _make_ipc_runtime_deps(),
    )


def _initialize_ipc_client(handle: int, owner_clients: dict[str, str], timeout_sec: float = 3.0) -> str:
    return ipc_runtime.initialize_ipc_client(
        handle,
        owner_clients,
        timeout_sec,
        _make_ipc_runtime_deps(),
    )


def _discover_owner_client_for_thread(handle: int, thread_id: str, timeout_sec: float) -> str | None:
    return ipc_runtime.discover_owner_client_for_thread(
        handle,
        thread_id,
        timeout_sec,
        _make_ipc_runtime_deps(),
    )


def _make_ipc_runtime_deps() -> ipc_runtime.IpcRuntimeDeps:
    return ipc_runtime.IpcRuntimeDeps(
        read_ipc_message=_read_ipc_message,
        write_ipc_message=_write_ipc_message,
        read_ipc_response=_read_ipc_response,
        uuid_factory=lambda: str(uuid.uuid4()),
        time_now=time.time,
        pipe_peek_retry_sec=PIPE_PEEK_RETRY_SEC,
    )


IPCNoClientFoundError = ipc_runtime.IPCNoClientFoundError


def _request_start_turn_via_ipc(
    handle: int,
    source_client_id: str,
    thread: ThreadInfo,
    prompt: str,
    timeout_sec: float,
    owner_clients: dict[str, str],
) -> dict[str, str]:
    return ipc_runtime.request_start_turn_via_ipc(
        handle, source_client_id, thread, prompt, timeout_sec, owner_clients, _make_ipc_runtime_deps()
    )


__all__ = (
    "IPCNoClientFoundError",
    "_busy_state_ensure_thread_loaded",
    "_close_ipc_pipe",
    "_discover_owner_client_for_thread",
    "_extract_thread_snapshot_from_ipc_message",
    "_initialize_ipc_client",
    "_make_busy_state_deps",
    "_make_ipc_runtime_deps",
    "_open_codex_ipc_pipe",
    "_read_ipc_message",
    "_read_ipc_response",
    "_record_owner_client_from_ipc_message",
    "_request_start_turn_via_ipc",
    "_write_ipc_message",
    "build_interactive_notice_from_function_call",
    "classify_thread_status",
    "classify_interactive_function_call",
    "describe_thread_busy_state",
    "describe_thread_context_usage",
    "extract_message_text",
    "format_thread_model_display",
    "get_busy_threads",
    "get_high_context_threads",
    "get_last_user_and_assistant_messages",
    "get_orphan_task_started_grace_seconds",
    "get_pending_interactive_display_lines",
    "get_pending_interactive_function_call_from_session",
    "get_pending_interactive_state_from_session",
    "get_pending_interactive_summary",
    "get_pending_permission_approval_from_session",
    "get_stale_busy_session_seconds",
    "get_thread_busy_state",
    "get_thread_collaboration_mode",
    "get_thread_context_usage",
    "get_thread_service_tier",
    "is_thread_busy",
    "parse_function_call_arguments",
    "read_new_session_events",
    "session_file_age_seconds",
    "should_recommend_archive",
    "summarize_interactive_lines",
)

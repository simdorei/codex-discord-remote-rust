from __future__ import annotations

from codex_desktop_bridge_impl_common import *

import codex_desktop_bridge_final_answer_transport as final_answer_transport

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import build_approval_decision_payload, build_interactive_notice_from_function_call, build_reply_input_response_payload, clear_cached_live_approval_request, detect_running_codex_desktop_executable, extract_message_text, get_cached_live_approval_request, get_pending_approval_request_via_ipc, get_pending_permission_approval_from_session, get_pending_user_input_request_via_ipc, get_thread_busy_state, load_recent_threads, make_console_safe_text, read_new_session_events, submit_approval_decision_via_ipc, submit_permission_approval_via_ui_row_select, submit_user_input_via_ipc

def _make_pending_reply_deps() -> bridge_reply.PendingReplyDeps:
    return bridge_reply.PendingReplyDeps(
        get_pending_user_input_request=get_pending_user_input_request_via_ipc,
        get_pending_approval_request=get_pending_approval_request_via_ipc,
        get_thread_busy_state=lambda thread, allow_resume: get_thread_busy_state(thread, allow_resume=allow_resume),
        build_reply_input_response_payload=build_reply_input_response_payload,
        submit_user_input=lambda thread, request_id, response_payload, timeout_sec: submit_user_input_via_ipc(
            thread,
            request_id,
            response_payload,
            timeout_sec=timeout_sec,
        ),
        get_cached_live_approval_request=get_cached_live_approval_request,
        get_pending_permission_approval_from_session=get_pending_permission_approval_from_session,
        submit_permission_approval_via_ui_row_select=submit_permission_approval_via_ui_row_select,
        build_approval_decision_payload=build_approval_decision_payload,
        build_approval_decision_candidate_payloads=reply_payload.build_approval_decision_candidate_payloads,
        submit_approval_decision=lambda thread, request_id, decision_payload, request_kind, timeout_sec, use_target: (
            submit_approval_decision_via_ipc(
                thread,
                request_id,
                decision_payload,
                request_kind,
                timeout_sec=timeout_sec,
                use_target_client=use_target,
            )
        ),
        clear_cached_live_approval_request=clear_cached_live_approval_request,
        time_now=time.time,
        sleep=time.sleep,
    )

is_windowsapps_path = sidecar_transport.is_windowsapps_path

detect_running_codex_app_server_executable = sidecar_transport.detect_running_codex_app_server_executable

iter_codex_app_server_bin_candidates = sidecar_transport.iter_codex_app_server_bin_candidates

resolve_codex_app_server_executable = sidecar_transport.resolve_codex_app_server_executable

def discover_codex_desktop_executable() -> tuple[Path | None, str]:
    return desktop_process.discover_codex_desktop_executable(
        env_name=CODEX_DESKTOP_EXE_ENV,
        deps=_make_desktop_process_deps(),
    )

def read_codex_app_package_version() -> tuple[str | None, str]:
    if os.name != "nt":
        return None, "skipped: OpenAI.Codex AppX package check is Windows-only"

    try:
        completed = subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-Command",
                "(Get-AppxPackage -Name OpenAI.Codex).Version",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
            check=False,
        )
    except OSError as exc:
        return None, f"Get-AppxPackage OpenAI.Codex failed: {exc}"

    details = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
    if completed.returncode != 0:
        return None, details or f"Get-AppxPackage OpenAI.Codex failed with exit {completed.returncode}"

    for line in completed.stdout.splitlines():
        version = line.strip()
        if version:
            return version, "Get-AppxPackage OpenAI.Codex"
    return None, "Get-AppxPackage OpenAI.Codex returned no version"

def check_codex_app_update() -> doctor_report.CodexAppUpdateStatus:
    current_version, details = read_codex_app_package_version()
    record = bridge_state.record_codex_app_package_version(current_version)
    return doctor_report.CodexAppUpdateStatus(
        current_version=record.current_version,
        previous_version=record.previous_version,
        update_detected=record.update_detected,
        details=details,
    )

def ensure_codex_desktop_executable_configured() -> tuple[Path, str, bool]:
    return desktop_process.ensure_codex_desktop_executable_configured(
        bridge_env_path=BRIDGE_ENV_PATH,
        env_name=CODEX_DESKTOP_EXE_ENV,
        deps=_make_desktop_process_deps(),
    )

def stop_codex_desktop_processes(executable_path: Path) -> tuple[bool, str]:
    return desktop_process.stop_codex_desktop_processes(
        executable_path,
        deps=_make_desktop_process_deps(),
    )

def stop_codex_archive_lock_candidates() -> list[str]:
    lines: list[str] = []
    desktop_exe, desktop_source = discover_codex_desktop_executable()
    if desktop_exe is None:
        lines.append("codex_desktop_stop: skipped; executable not discovered")
    else:
        stopped, details = stop_codex_desktop_processes(desktop_exe)
        lines.append(f"codex_desktop_stop: stopped={stopped} source={desktop_source or '-'} exe={desktop_exe}")
        lines.append(f"codex_desktop_stop_details: {make_console_safe_text(details)}")

    stopped_app_servers, app_server_details = desktop_process.stop_codex_app_server_processes()
    lines.append(f"codex_app_server_stop: stopped={stopped_app_servers}")
    lines.append(f"codex_app_server_stop_details: {make_console_safe_text(app_server_details)}")
    return lines

def start_codex_desktop_process(executable_path: Path) -> desktop_process.StartedDesktopProcess:
    return desktop_process.start_codex_desktop_process(
        executable_path,
        deps=_make_desktop_process_deps(),
    )

def _make_desktop_process_deps() -> desktop_process.DesktopProcessDeps:
    return desktop_process.DesktopProcessDeps(
        get_optional_env_file_path=get_optional_env_file_path,
        detect_running_codex_desktop_executable=detect_running_codex_desktop_executable,
        detect_codex_desktop_executable_via_powershell=desktop_resolver.detect_codex_desktop_executable_via_powershell,
        iter_codex_desktop_registry_candidates=desktop_resolver.iter_codex_desktop_registry_candidates,
        iter_default_codex_desktop_candidates=desktop_resolver.iter_default_codex_desktop_candidates,
        persist_env_value=desktop_resolver.persist_env_value,
        set_environ_value=lambda name, value: os.environ.__setitem__(name, value),
        which=shutil.which,
        run_process=subprocess.run,
        start_process=subprocess.Popen,
        create_no_window=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        create_new_process_group=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0),
    )

def archive_thread_with_lock_retry(thread_id: str, *, kill_codex_on_lock: bool) -> None:
    archive_retry.archive_thread_with_lock_retry(
        thread_id,
        kill_codex_on_lock=kill_codex_on_lock,
        stop_lock_candidates=stop_codex_archive_lock_candidates,
        archive_once=archive_thread_once,
    )

archive_thread_once = archive_retry.archive_thread_once

load_thread_record_by_id = thread_records.load_thread_record_by_id

wait_for_thread_record = thread_records.wait_for_thread_record

resolve_new_thread_cwd = new_thread.resolve_new_thread_cwd

def start_turn_via_sidecar(
    thread: ThreadInfo,
    prompt: str,
    *,
    timeout_sec: float = 10.0,
    keep_client_open: bool = False,
) -> sidecar_thread.StartTurnResult:
    return sidecar_thread.start_turn_via_sidecar(
        thread,
        prompt,
        timeout_sec=timeout_sec,
        keep_client_open=keep_client_open,
        deps=_make_start_turn_sidecar_deps(),
    )

def _make_start_turn_sidecar_deps() -> sidecar_thread.StartTurnSidecarDeps:
    return sidecar_thread.StartTurnSidecarDeps(
        new_sidecar=CodexAppServerSidecar,
        ensure_thread_loaded=sidecar_thread.ensure_thread_loaded_via_sidecar,
        is_transient_sidecar_attach_error=sidecar_thread.is_transient_sidecar_attach_error,
        time_now=time.time,
        sleep=time.sleep,
    )

spawn_background_new_thread_runner = new_thread.spawn_background_new_thread_runner

normalize_prompt_text = prompt_delivery.normalize_prompt_text

def snapshot_recent_session_offsets(
    limit: int = 10,
    include_threads: list[ThreadInfo] | None = None,
) -> prompt_delivery.SessionOffsets:
    return prompt_delivery.snapshot_recent_session_offsets(
        limit=limit,
        include_threads=include_threads,
        load_recent_threads=lambda recent_limit: load_recent_threads(limit=recent_limit),
    )

def wait_for_prompt_delivery(
    session_offsets: prompt_delivery.SessionOffsets,
    prompt: str,
    timeout_sec: float = 4.0,
) -> ThreadInfo | None:
    return prompt_delivery.wait_for_prompt_delivery(
        session_offsets,
        prompt,
        timeout_sec,
        read_new_session_events=read_new_session_events,
        extract_message_text=extract_message_text,
        time_now=time.time,
        sleep=time.sleep,
    )

def emit_watch_stream_block(
    marker: str,
    text: str,
    *,
    stream_label: str = "",
    stream_callback: Callable[[str], None] | None = None,
) -> None:
    lock = nullcontext() if stream_callback is not None else PRINT_LOCK
    with lock:
        final_answer_watch.emit_watch_stream_block(
            marker,
            text,
            stream_label=stream_label,
            stream_callback=stream_callback,
        )

def _make_final_answer_watch_deps() -> final_answer_watch.FinalAnswerWatchDeps:
    return final_answer_watch.FinalAnswerWatchDeps(
        time_now=time.time,
        sleep=time.sleep,
        read_new_session_events=read_new_session_events,
        build_interactive_notice_from_function_call=build_interactive_notice_from_function_call,
        extract_message_text=extract_message_text,
        emit_watch_stream_block=emit_watch_stream_block,
        get_thread_goal_status=final_answer_transport.get_thread_goal_status,
        observe_turn_completion=final_answer_transport.observe_turn_completion,
        get_thread_goal_lookup=final_answer_transport.get_thread_goal_lookup,
        get_thread_goal_update=final_answer_transport.get_thread_goal_update,
    )

__all__ = ('_make_desktop_process_deps', '_make_final_answer_watch_deps', '_make_pending_reply_deps', '_make_start_turn_sidecar_deps', 'annotations', 'archive_thread_once', 'archive_thread_with_lock_retry', 'check_codex_app_update', 'detect_running_codex_app_server_executable', 'discover_codex_desktop_executable', 'emit_watch_stream_block', 'ensure_codex_desktop_executable_configured', 'is_windowsapps_path', 'iter_codex_app_server_bin_candidates', 'load_thread_record_by_id', 'normalize_prompt_text', 'read_codex_app_package_version', 'resolve_codex_app_server_executable', 'resolve_new_thread_cwd', 'snapshot_recent_session_offsets', 'spawn_background_new_thread_runner', 'start_codex_desktop_process', 'start_turn_via_sidecar', 'stop_codex_archive_lock_candidates', 'stop_codex_desktop_processes', 'wait_for_prompt_delivery', 'wait_for_thread_record')

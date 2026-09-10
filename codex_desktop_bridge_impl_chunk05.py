from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import typed_choose_thread_from_args as _choose_thread_from_args, typed_make_new_command_deps as _make_new_command_deps, build_workspace_ref_map, check_codex_app_update, choose_thread, click_window, collapse_list_text, describe_thread_context_usage, discover_codex_desktop_executable, ensure_codex_composer_focus, ensure_codex_desktop_executable_configured, find_codex_window, focus_window, format_thread_model_display, format_timestamp, format_title_preview, format_token_k, get_busy_threads, get_clipboard_text, get_high_context_threads, get_last_user_and_assistant_messages, get_live_pending_approval_display_lines, get_pending_interactive_summary, get_saved_thread_settings, get_selected_thread_id, get_thread_busy_state, get_thread_collaboration_mode, get_thread_context_usage, get_thread_service_tier, get_thread_slot, get_thread_ui_name, get_thread_workspace_name, get_thread_workspace_ref, is_protocol_registered, is_thread_busy, load_archived_threads, load_ordinary_user_root_threads, load_recent_threads, load_user_root_threads, make_console_safe_text, remember_thread_settings, resolve_thread_ref, send_hotkey, send_key_event, set_clipboard_text, should_recommend_archive, start_codex_desktop_process, stop_codex_desktop_processes, summarize_interactive_lines

def _make_prompt_sender_deps() -> prompt_sender.PromptSenderDeps:
    return prompt_sender.PromptSenderDeps(
        find_codex_window=find_codex_window,
        focus_window=focus_window,
        ensure_codex_composer_focus=ensure_codex_composer_focus,
        click_window=click_window,
        send_hotkey=send_hotkey,
        send_key_event=lambda vk, keyup: send_key_event(vk, keyup=keyup),
        set_clipboard_text=set_clipboard_text,
        get_clipboard_text=get_clipboard_text,
        sleep=time.sleep,
        print_line=print,
        vk_control=VK_CONTROL,
        vk_a=VK_A,
        vk_back=VK_BACK,
        vk_v=VK_V,
        vk_return=VK_RETURN,
    )

def print_thread_list(threads: list[ThreadInfo]) -> None:
    thread_list.print_thread_list(threads, _make_thread_list_deps())

def print_archived_thread_list(threads: list[ThreadInfo]) -> None:
    thread_list.print_archived_thread_list(threads, _make_thread_list_deps())

def _make_thread_list_deps() -> thread_list.ThreadListDeps:
    return thread_list.ThreadListDeps(
        get_selected_thread_id=get_selected_thread_id,
        build_workspace_ref_map=build_workspace_ref_map,
        get_thread_ui_name=get_thread_ui_name,
        collapse_list_text=collapse_list_text,
        get_thread_workspace_name=get_thread_workspace_name,
        is_thread_busy=is_thread_busy,
        new_sidecar=CodexAppServerSidecar,
        get_thread_busy_state=lambda thread, client, allow_resume: get_thread_busy_state(
            thread,
            client=client,
            allow_resume=allow_resume,
        ),
        get_thread_context_usage=get_thread_context_usage,
        format_token_k=format_token_k,
        should_recommend_archive=should_recommend_archive,
        format_thread_model_display=format_thread_model_display,
        get_thread_collaboration_mode=get_thread_collaboration_mode,
        get_thread_service_tier=get_thread_service_tier,
        format_timestamp=format_timestamp,
        make_console_safe_text=make_console_safe_text,
        get_live_pending_approval_display_lines=lambda thread, timeout_sec: (
            get_live_pending_approval_display_lines(thread, timeout_sec=timeout_sec)
        ),
        summarize_interactive_lines=summarize_interactive_lines,
        get_pending_interactive_summary=get_pending_interactive_summary,
        print_line=print,
    )

def command_list(args: argparse.Namespace) -> int:
    limit = _arg_int(args, "limit", 50)
    if _arg_bool(args, "db_root"):
        threads = load_ordinary_user_root_threads(limit=limit)
    else:
        threads = load_recent_threads(limit=limit)
    print_thread_list(threads)
    return 0

def command_settings(args: argparse.Namespace) -> int:
    settings_commands.run_settings_command(
        thread_ref=_arg_text(args, "thread_ref").strip(),
        thread_id=_arg_optional_text(args, "thread_id"),
        cwd=_arg_optional_text(args, "cwd"),
        model=_arg_optional_text(args, "model"),
        reasoning=_arg_optional_text(args, "reasoning"),
        speed=_arg_optional_text(args, "speed"),
        deps=_make_settings_command_deps(),
    )
    return 0

def command_settings_options(args: argparse.Namespace) -> int:
    settings_commands.run_settings_options_command(
        field=_arg_text(args, "field", "all"),
        deps=_make_settings_command_deps(),
    )
    return 0

def _make_settings_command_deps() -> settings_commands.SettingsCommandDeps:
    return settings_commands.SettingsCommandDeps(
        choose_thread=choose_thread,
        resolve_thread_ref=resolve_thread_ref,
        new_sidecar=CodexAppServerSidecar,
        build_thread_settings_update=build_thread_settings_update,
        remember_thread_settings=remember_thread_settings,
        get_saved_thread_settings=get_saved_thread_settings,
        format_title_preview=format_title_preview,
        format_settings_options=format_settings_options,
        print_line=print,
    )

def command_archived_list(args: argparse.Namespace) -> int:
    threads = load_archived_threads(limit=_arg_int(args, "limit", 50))
    print_archived_thread_list(threads)
    return 0

def command_status(args: argparse.Namespace) -> int:
    thread = _choose_thread_from_args(args)
    status_report.print_thread_status(thread, _make_status_report_deps())
    return 0

def _make_status_report_deps() -> status_report.StatusReportDeps:
    return status_report.StatusReportDeps(
        get_last_user_and_assistant_messages=get_last_user_and_assistant_messages,
        get_thread_busy_state=lambda thread: get_thread_busy_state(thread, allow_resume=True),
        get_thread_slot=get_thread_slot,
        get_thread_ui_name=get_thread_ui_name,
        get_thread_context_usage=get_thread_context_usage,
        get_thread_workspace_ref=get_thread_workspace_ref,
        format_timestamp=format_timestamp,
        describe_thread_context_usage=describe_thread_context_usage,
        print_line=print,
    )

def command_doctor(args: argparse.Namespace) -> int:
    doctor_report.print_doctor_report(
        _arg_int(args, "limit", 10),
        _make_doctor_report_config(),
        _make_doctor_report_deps(),
    )
    return 0

def _make_doctor_report_config() -> doctor_report.DoctorReportConfig:
    return doctor_report.DoctorReportConfig(
        platform_text=platform.platform(),
        python_version=platform.python_version(),
        python_executable=sys.executable,
        codex_home=CODEX_HOME,
        state_db_path=STATE_DB_PATH,
        session_index_path=SESSION_INDEX_PATH,
        global_state_path=GLOBAL_STATE_PATH,
        bridge_state_path=BRIDGE_STATE_PATH,
        high_context_input_ratio_threshold=HIGH_CONTEXT_INPUT_RATIO_THRESHOLD,
    )

def _make_doctor_report_deps() -> doctor_report.DoctorReportDeps:
    return doctor_report.DoctorReportDeps(
        is_protocol_registered=is_protocol_registered,
        get_selected_thread_id=get_selected_thread_id,
        active_thread_count=_count_active_threads_for_doctor,
        get_high_context_threads=lambda limit: get_high_context_threads(limit=limit),
        get_thread_workspace_ref=get_thread_workspace_ref,
        find_codex_window=find_codex_window,
        make_console_safe_text=make_console_safe_text,
        get_busy_threads=lambda limit: get_busy_threads(limit=limit),
        discover_codex_desktop_executable=discover_codex_desktop_executable,
        check_codex_app_update=check_codex_app_update,
        print_line=print,
    )

def _count_active_threads_for_doctor() -> tuple[int, str]:
    if not STATE_DB_PATH.exists():
        return 0, ""
    try:
        return bridge_sqlite.count_active_threads(STATE_DB_PATH), ""
    except (sqlite3.Error, OSError, RuntimeError) as exc:
        return 0, str(exc)

def command_discover_codex(_args: argparse.Namespace) -> int:
    desktop_commands.run_discover_codex_command(_make_desktop_command_deps())
    return 0

def command_restart_codex(args: argparse.Namespace) -> int:
    desktop_commands.run_restart_codex_command(
        stop_wait=_arg_float(args, "stop_wait"),
        start_wait=_arg_float(args, "start_wait"),
        deps=_make_desktop_command_deps(),
    )
    return 0

def command_focus(args: argparse.Namespace) -> int:
    desktop_commands.run_focus_command(
        click=_arg_bool(args, "click"),
        click_x_ratio=_arg_float(args, "click_x_ratio"),
        click_y_offset=_arg_int(args, "click_y_offset"),
        deps=_make_desktop_command_deps(),
    )
    return 0

def _make_desktop_command_deps() -> desktop_commands.DesktopCommandDeps:
    return desktop_commands.DesktopCommandDeps(
        ensure_codex_desktop_executable_configured=ensure_codex_desktop_executable_configured,
        stop_codex_desktop_processes=stop_codex_desktop_processes,
        start_codex_desktop_process=start_codex_desktop_process,
        find_codex_window=find_codex_window,
        focus_window=focus_window,
        ensure_codex_composer_focus=ensure_codex_composer_focus,
        click_window=click_window,
        make_console_safe_text=make_console_safe_text,
        sleep=time.sleep,
        print_line=print,
        bridge_env_path=BRIDGE_ENV_PATH,
    )

def command_new(args: argparse.Namespace) -> int:
    new_command.run_new_command(
        cwd=_arg_optional_text(args, "cwd"),
        prompt=_arg_optional_text(args, "prompt"),
        abort=_arg_bool(args, "abort"),
        create_timeout=_arg_float(args, "create_timeout"),
        deps=_make_new_command_deps(),
    )
    return 0

__all__ = ('_count_active_threads_for_doctor', '_make_desktop_command_deps', '_make_doctor_report_config', '_make_doctor_report_deps', '_make_prompt_sender_deps', '_make_settings_command_deps', '_make_status_report_deps', '_make_thread_list_deps', 'annotations', 'command_archived_list', 'command_discover_codex', 'command_doctor', 'command_focus', 'command_list', 'command_new', 'command_restart_codex', 'command_settings', 'command_settings_options', 'command_status', 'print_archived_thread_list', 'print_thread_list')

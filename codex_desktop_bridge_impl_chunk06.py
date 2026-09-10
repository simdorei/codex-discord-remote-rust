from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import typed_choose_or_resolve_thread_from_args as _choose_or_resolve_thread_from_args, typed_choose_thread_from_args as _choose_thread_from_args, activate_thread_in_ui, archive_thread_with_lock_retry, cancel_codex_reply_if_busy, choose_thread, command_archived_list, command_discover_codex, command_doctor, command_focus, command_list, command_new, command_restart_codex, command_settings, command_settings_options, command_status, describe_thread_busy_state, extract_message_text, format_timestamp, format_title_preview, get_busy_threads, get_last_user_and_assistant_messages, get_live_pending_approval_display_lines, get_pending_interactive_display_lines, get_selected_thread_id, get_thread_busy_state, get_thread_label, get_thread_ui_name, get_thread_workspace_ref, load_recent_threads, make_console_safe_text, read_new_session_events, reply_to_pending_approval, resolve_archived_thread_ref, resolve_new_thread_cwd, resolve_thread_ref, rotate_single_backup_file, send_prompt_to_codex, set_selected_thread_id, snapshot_recent_session_offsets, spawn_background_new_thread_runner, start_background_watch, start_turn_via_ipc, start_turn_via_sidecar, sync_session_index_with_state, verify_thread_in_ui, wait_for_new_thread, wait_for_prompt_delivery, wait_for_thread_record, watch_for_final_answer

def _make_new_command_deps() -> new_command.NewCommandDeps:
    return new_command.NewCommandDeps(
        cancel_codex_reply_if_busy=lambda timeout_sec: cancel_codex_reply_if_busy(timeout_sec=timeout_sec),
        resolve_new_thread_cwd=resolve_new_thread_cwd,
        load_recent_threads=lambda limit: load_recent_threads(limit=limit),
        spawn_background_new_thread_runner=spawn_background_new_thread_runner,
        wait_for_new_thread=lambda previous_ids, timeout_sec: wait_for_new_thread(
            previous_ids,
            timeout_sec=timeout_sec,
        ),
        set_selected_thread_id=set_selected_thread_id,
        format_title_preview=format_title_preview,
        get_thread_ui_name=get_thread_ui_name,
        wait_for_prompt_delivery=lambda session_offsets, prompt, timeout_sec: wait_for_prompt_delivery(
            session_offsets,
            prompt,
            timeout_sec=timeout_sec,
        ),
        get_thread_label=get_thread_label,
        sync_session_index_with_state=sync_session_index_with_state,
        print_line=print,
    )

def command_archive(args: argparse.Namespace) -> int:
    archive_commands.run_archive_command(
        _choose_or_resolve_thread_from_args(args),
        no_kill_codex_on_lock=_arg_bool(args, "no_kill_codex_on_lock"),
        timeout=_arg_float(args, "timeout"),
        deps=_make_archive_command_deps(),
    )
    return 0

def command_delete_archive(args: argparse.Namespace) -> int:
    thread = resolve_archived_thread_ref(_arg_text(args, "thread_ref"), limit=0)
    archive_commands.run_delete_archive_command(
        thread,
        confirm=_arg_bool(args, "confirm"),
        deps=_make_archive_command_deps(),
    )
    return 0

def _make_archive_command_deps() -> archive_commands.ArchiveCommandDeps:
    return archive_commands.ArchiveCommandDeps(
        get_thread_busy_state=lambda thread: get_thread_busy_state(thread, allow_resume=True),
        describe_thread_busy_state=describe_thread_busy_state,
        archive_thread_with_lock_retry=lambda thread_id, kill_codex_on_lock: (
            archive_thread_with_lock_retry(thread_id, kill_codex_on_lock=kill_codex_on_lock)
        ),
        wait_for_thread_record=lambda thread_id, archived, timeout: wait_for_thread_record(
            thread_id,
            archived=archived,
            timeout_sec=timeout,
        ),
        get_selected_thread_id=get_selected_thread_id,
        set_selected_thread_id=set_selected_thread_id,
        sync_session_index_with_state=sync_session_index_with_state,
        format_title_preview=format_title_preview,
        format_timestamp=format_timestamp,
        delete_archived_thread_locally=archive_delete.delete_archived_thread_locally,
        print_line=print,
    )

def command_use(args: argparse.Namespace) -> int:
    if _arg_bool(args, "clear"):
        set_selected_thread_id(None)
        print("selected_thread: cleared")
        return 0

    thread = _choose_or_resolve_thread_from_args(args)
    set_selected_thread_id(thread.id)
    use_report.print_use_report(thread, _make_use_report_deps())
    return 0

def _make_use_report_deps() -> use_report.UseReportDeps:
    return use_report.UseReportDeps(
        get_last_user_and_assistant_messages=get_last_user_and_assistant_messages,
        get_thread_busy_state=lambda thread: get_thread_busy_state(thread, allow_resume=True),
        get_live_pending_approval_display_lines=lambda thread, timeout_sec: (
            get_live_pending_approval_display_lines(thread, timeout_sec=timeout_sec)
        ),
        get_pending_interactive_display_lines=get_pending_interactive_display_lines,
        format_title_preview=format_title_preview,
        get_thread_ui_name=get_thread_ui_name,
        print_line=print,
    )

def command_approval_reply(args: argparse.Namespace) -> int:
    approval_report.run_approval_reply_command(
        thread_ref=_arg_text(args, "thread_ref").strip(),
        thread_id=_arg_optional_text(args, "thread_id"),
        cwd=_arg_optional_text(args, "cwd"),
        answer=_arg_text(args, "answer"),
        timeout=_arg_float(args, "timeout"),
        deps=_make_approval_reply_command_deps(),
    )
    return 0

def _make_approval_reply_command_deps() -> approval_report.ApprovalReplyCommandDeps:
    return approval_report.ApprovalReplyCommandDeps(
        choose_thread=choose_thread,
        resolve_thread_ref=resolve_thread_ref,
        reply_to_pending_approval=reply_to_pending_approval,
        get_thread_workspace_ref=get_thread_workspace_ref,
        print_line=print,
    )

def command_tail(args: argparse.Namespace) -> int:
    thread = _choose_thread_from_args(args)
    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        raise SessionFileMissingError(session_path)
    bridge_tail.tail_session_events(
        session_path,
        only_new=_arg_bool(args, "only_new"),
        timeout=_arg_float(args, "timeout"),
        deps=_make_tail_deps(),
    )
    return 0

def _make_tail_deps() -> bridge_tail.TailDeps:
    return bridge_tail.TailDeps(
        read_new_session_events=read_new_session_events,
        extract_message_text=extract_message_text,
        time_now=time.time,
        sleep=time.sleep,
        print_line=print,
    )

def command_open(args: argparse.Namespace) -> int:
    open_command.run_open_command(
        _choose_or_resolve_thread_from_args(args),
        abort=_arg_bool(args, "abort"),
        deps=_make_open_command_deps(),
    )
    return 0

def _make_open_command_deps() -> open_command.OpenCommandDeps:
    return open_command.OpenCommandDeps(
        get_busy_threads=lambda limit: get_busy_threads(limit=limit),
        get_thread_label=get_thread_label,
        cancel_codex_reply_if_busy=lambda timeout_sec: cancel_codex_reply_if_busy(timeout_sec=timeout_sec),
        set_selected_thread_id=set_selected_thread_id,
        activate_thread_in_ui=activate_thread_in_ui,
        get_last_user_and_assistant_messages=get_last_user_and_assistant_messages,
        format_title_preview=format_title_preview,
        get_thread_ui_name=get_thread_ui_name,
        print_line=print,
    )

def command_stop(args: argparse.Namespace) -> int:
    stop_command.run_stop_command(
        _choose_or_resolve_thread_from_args(args),
        deps=_make_stop_command_deps(),
    )
    return 0

def _make_stop_command_deps() -> stop_command.StopCommandDeps:
    return stop_command.StopCommandDeps(
        get_active_turn_id=sidecar_thread.get_active_turn_id_via_app_server_or_raise,
        interrupt_turn=sidecar_thread.interrupt_turn_via_app_server,
        get_thread_label=get_thread_label,
        time_now=time.time,
        sleep=time.sleep,
        print_line=print,
    )

def _make_command_ask_deps() -> command_ask_types.CommandAskDeps:
    return command_ask_types.CommandAskDeps(
        choose_thread=choose_thread,
        format_title_preview=format_title_preview,
        get_thread_ui_name=get_thread_ui_name,
        get_thread_busy_state=get_thread_busy_state,
        describe_thread_busy_state=describe_thread_busy_state,
        snapshot_recent_session_offsets=snapshot_recent_session_offsets,
        wait_for_prompt_delivery=wait_for_prompt_delivery,
        start_turn_via_sidecar=start_turn_via_sidecar,
        start_turn_via_ipc=start_turn_via_ipc,
        activate_thread_in_ui=activate_thread_in_ui,
        verify_thread_in_ui=verify_thread_in_ui,
        send_prompt_to_codex=send_prompt_to_codex,
        start_background_watch=start_background_watch,
        watch_for_final_answer=watch_for_final_answer,
        get_thread_label=get_thread_label,
        make_console_safe_text=make_console_safe_text,
    )

def command_ask(args: argparse.Namespace) -> int:
    return command_ask_runner.run_command_ask(args, deps=_make_command_ask_deps())

def make_cli_handlers() -> bridge_cli.BridgeCommandHandlers:
    return bridge_cli.BridgeCommandHandlers(
        command_list=command_list,
        command_settings=command_settings,
        command_settings_options=command_settings_options,
        command_archived_list=command_archived_list,
        command_status=command_status,
        command_doctor=command_doctor,
        command_discover_codex=command_discover_codex,
        command_restart_codex=command_restart_codex,
        command_focus=command_focus,
        command_new=command_new,
        command_archive=command_archive,
        command_delete_archive=command_delete_archive,
        command_use=command_use,
        command_approval_reply=command_approval_reply,
        command_tail=command_tail,
        command_open=command_open,
        command_stop=command_stop,
        command_ask=command_ask,
    )

def build_parser() -> argparse.ArgumentParser:
    return bridge_cli.build_parser(make_cli_handlers())

split_repl_command = bridge_cli.split_repl_command

def run_repl() -> int:
    return bridge_repl.run_repl(_make_repl_deps())

def _make_repl_deps() -> bridge_repl.ReplDeps:
    return bridge_repl.ReplDeps(
        get_selected_thread_id=get_selected_thread_id,
        build_parser=build_parser,
        split_repl_command=split_repl_command,
        input_line=input,
        print_line=print,
    )

def main() -> int:
    rotate_single_backup_file(IPC_PROBE_LOG_PATH)
    if len(sys.argv) == 1:
        return run_repl()

    parser = build_parser()
    args = parser.parse_args()
    try:
        func = bridge_cli.require_command_func(args, parser)
        return int(func(args))
    except KeyboardInterrupt:
        print("Interrupted.", file=sys.stderr)
        return 130
    except (argparse.ArgumentError, OSError, RuntimeError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1

__all__ = ('_make_approval_reply_command_deps', '_make_archive_command_deps', '_make_command_ask_deps', '_make_new_command_deps', '_make_open_command_deps', '_make_repl_deps', '_make_stop_command_deps', '_make_tail_deps', '_make_use_report_deps', 'annotations', 'build_parser', 'command_approval_reply', 'command_archive', 'command_ask', 'command_delete_archive', 'command_open', 'command_stop', 'command_tail', 'command_use', 'main', 'make_cli_handlers', 'run_repl', 'split_repl_command')

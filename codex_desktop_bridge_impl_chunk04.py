from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import typed_make_final_answer_watch_deps as _make_final_answer_watch_deps, typed_make_prompt_sender_deps as _make_prompt_sender_deps, get_busy_threads, get_selected_thread_id, get_thread_label, get_thread_ui_name_candidates, load_recent_threads

def watch_for_final_answer(
    session_path: Path,
    start_offset: int,
    timeout_sec: float,
    include_commentary: bool,
    stream_live: bool = False,
    stream_label: str = "",
    stream_callback: Callable[[str], None] | None = None,
    expected_turn_id: str | None = None,
) -> final_answer_watch.WatchForFinalAnswerResult:
    return final_answer_watch.watch_for_final_answer(
        session_path,
        start_offset,
        timeout_sec,
        include_commentary,
        deps=_make_final_answer_watch_deps(),
        stream_live=stream_live,
        stream_label=stream_label,
        stream_callback=stream_callback,
        expected_turn_id=expected_turn_id,
    )

def _print_background_watch_status(label: str, status: str) -> None:
    with PRINT_LOCK:
        print(f"{label} [{status}]")
        print("")

def start_background_watch(
    thread: ThreadInfo,
    start_offset: int,
    timeout_sec: float,
    include_commentary: bool,
    stream_output: bool,
) -> bool:
    return background_watch.start_background_watch(
        thread,
        start_offset,
        timeout_sec,
        include_commentary,
        stream_output,
        background_watch.BackgroundWatchDeps(
            get_thread_label=get_thread_label,
            watch_for_final_answer=watch_for_final_answer,
            print_watch_status=_print_background_watch_status,
        ),
    )

def get_window_text(hwnd: int) -> str:
    return window_focus.get_window_text(hwnd, _make_window_text_deps())

is_codex_desktop_window_title = window_focus.is_codex_desktop_window_title

def find_codex_window() -> WindowInfo:
    return window_focus.find_codex_window(_make_window_focus_deps())

def focus_window(window: WindowInfo) -> None:
    window_focus.focus_window(window, _make_window_focus_deps())

def ensure_codex_composer_focus(attempts: int = 4) -> bool:
    return window_focus.ensure_codex_composer_focus(attempts, _make_window_focus_deps())

def _make_window_text_deps() -> window_focus.WindowTextDeps:
    return window_focus.WindowTextDeps(
        get_window_text_length=windows_input.get_window_text_length,
        read_window_text=windows_input.read_window_text,
    )

def _make_window_focus_deps() -> window_focus.WindowFocusDeps:
    return window_focus.WindowFocusDeps(
        enum_windows=windows_input.enum_windows,
        is_window_visible=windows_input.is_window_visible,
        get_window_text=get_window_text,
        get_window_process_path=_get_window_process_path,
        get_window_rect=windows_input.get_window_rect_tuple,
        get_foreground_window=windows_input.get_foreground_window,
        show_window=lambda hwnd: windows_input.show_window(hwnd, SW_RESTORE),
        set_foreground_window=windows_input.set_foreground_window,
        bring_window_to_top=windows_input.bring_window_to_top,
        run_process=getattr(windows_input, "run_composer_focus_process", subprocess.run),
        send_key_event=lambda key, keyup: send_key_event(key, keyup=keyup),
        sleep=time.sleep,
        restore_command=SW_RESTORE,
        tab_key=VK_TAB,
        create_no_window=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )

def _get_window_process_path(hwnd: int) -> str:
    pid = desktop_resolver.get_window_process_id(hwnd)
    if not pid:
        return ""
    candidate = desktop_resolver.query_process_image_path(pid)
    return str(candidate) if candidate is not None else ""

def activate_thread_by_sidebar_v2(thread_name: str, project_name: str | None = None) -> str:
    return sidebar_activation.activate_thread_by_sidebar_v2(
        thread_name,
        project_name,
        _make_sidebar_activation_deps(),
    )

def _make_sidebar_activation_deps() -> sidebar_activation.SidebarActivationDeps:
    script_dir = Path(__file__).resolve().parent
    return sidebar_activation.SidebarActivationDeps(
        legacy_script_path=script_dir / "codex_desktop_bridge_sidebar_legacy.ps1",
        v2_script_path=script_dir / "codex_desktop_bridge_sidebar_v2.ps1",
        read_text=lambda path: path.read_text(encoding="utf-8"),
        find_codex_window=find_codex_window,
        focus_window=focus_window,
        run_process=getattr(windows_input, "run_sidebar_activation_process", subprocess.run),
        environ_copy=os.environ.copy,
        create_no_window=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )

get_clipboard_text = windows_input.get_clipboard_text

def verify_active_thread(thread_id: str) -> str | None:
    return active_thread.verify_active_thread(thread_id, _make_active_thread_deps())

def verify_active_thread_by_header(thread_name: str) -> str | None:
    return active_thread.verify_active_thread_by_header(thread_name, _make_active_thread_deps())

def _make_active_thread_deps() -> active_thread.ActiveThreadDeps:
    return active_thread.ActiveThreadDeps(
        get_clipboard_text=get_clipboard_text,
        set_clipboard_text=set_clipboard_text,
        find_codex_window=find_codex_window,
        focus_window=focus_window,
        send_hotkey=send_hotkey,
        send_key_event=send_key_event,
        sleep=time.sleep,
        time_ns=time.time_ns,
        run_process=getattr(windows_input, "run_header_verification_process", subprocess.run),
        environ_copy=os.environ.copy,
        vk_control=VK_CONTROL,
        vk_menu=VK_MENU,
        vk_l=VK_L,
        vk_c=VK_C,
        vk_escape=VK_ESCAPE,
        create_no_window=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )

def wait_for_thread_activation(thread: ThreadInfo, thread_name: str, timeout_sec: float = 5.0) -> str | None:
    return thread_activation.wait_for_thread_activation(
        thread,
        thread_name,
        timeout_sec=timeout_sec,
        deps=_make_thread_activation_deps(),
    )

def activate_thread_in_ui(thread: ThreadInfo) -> str:
    return thread_activation.activate_thread_in_ui(thread, _make_thread_activation_deps())

def activate_thread_for_ipc(thread: ThreadInfo) -> str:
    return thread_activation.activate_thread_via_deep_link(thread, _make_thread_activation_deps())

def verify_thread_in_ui(thread: ThreadInfo) -> str | None:
    return thread_activation.verify_thread_in_ui(thread, _make_thread_activation_deps())

def _make_thread_activation_deps() -> thread_activation.ThreadActivationDeps:
    return thread_activation.ThreadActivationDeps(
        get_thread_ui_name_candidates=get_thread_ui_name_candidates,
        verify_active_thread_by_header=verify_active_thread_by_header,
        verify_active_thread=verify_active_thread,
        activate_thread_deep_link=bridge_protocol.open_codex_thread_deep_link,
        activate_thread_by_sidebar_v2=activate_thread_by_sidebar_v2,
        wait_for_thread_activation=wait_for_thread_activation,
        now=time.time,
        sleep=time.sleep,
    )

set_clipboard_text = windows_input.set_clipboard_text

send_key_event = windows_input.send_key_event

send_hotkey = windows_input.send_hotkey

def wait_for_new_thread(previous_ids: set[str], timeout_sec: float = 8.0) -> ThreadInfo | None:
    return thread_actions.wait_for_new_thread(
        previous_ids,
        timeout_sec,
        _make_thread_action_deps(),
    )

def cancel_codex_reply_if_busy(timeout_sec: float = 3.0) -> tuple[list[str], list[str]]:
    return thread_actions.cancel_codex_reply_if_busy(
        timeout_sec,
        _make_thread_action_deps(),
    )

def _make_thread_action_deps() -> thread_actions.ThreadActionDeps:
    return thread_actions.ThreadActionDeps(
        load_recent_threads=lambda limit: load_recent_threads(limit=limit),
        get_busy_threads=lambda limit: get_busy_threads(limit=limit),
        get_thread_label=get_thread_label,
        get_selected_thread_id=get_selected_thread_id,
        interrupt_thread_via_sidecar=sidecar_thread.interrupt_thread_via_sidecar,
        time_now=time.time,
        sleep=time.sleep,
    )

click_window = windows_input.click_window

classify_permission_approval_ui_reply = permission_ui.classify_permission_approval_ui_reply

def submit_permission_approval_via_ui(answer_text: str) -> bridge_reply.ReplyResult:
    return permission_ui.submit_permission_approval_via_ui(answer_text, _make_permission_ui_deps())

def submit_permission_approval_via_ui_row_select(answer_text: str) -> bridge_reply.ReplyResult:
    return permission_ui.submit_permission_approval_via_ui_row_select(answer_text, _make_permission_ui_deps())

def _make_permission_ui_deps() -> permission_ui.PermissionUiDeps:
    return permission_ui.PermissionUiDeps(
        approval_script_path=SCRIPT_DIR / "codex_desktop_bridge_permission_approval.ps1",
        row_select_script_path=SCRIPT_DIR / "codex_desktop_bridge_permission_row_select.ps1",
        script_dir=SCRIPT_DIR,
        read_text=lambda script_path: script_path.read_text(encoding="utf-8"),
        get_clipboard_text=get_clipboard_text,
        set_clipboard_text=set_clipboard_text,
        find_codex_window=find_codex_window,
        focus_window=focus_window,
        send_hotkey=send_hotkey,
        send_key_event=send_key_event,
        sleep=time.sleep,
        run_process=getattr(windows_input, "run_permission_approval_process", subprocess.run),
        environ_copy=os.environ.copy,
        vk_control=VK_CONTROL,
        vk_v=VK_V,
        vk_return=VK_RETURN,
        create_no_window=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )

def send_prompt_to_codex(
    prompt: str,
    click_x_ratio: float,
    click_y_offset: int,
    skip_click: bool,
) -> WindowInfo:
    return prompt_sender.send_prompt_to_codex(
        prompt,
        click_x_ratio,
        click_y_offset,
        skip_click,
        _make_prompt_sender_deps(),
    )

__all__ = ('_make_active_thread_deps', '_make_permission_ui_deps', '_make_sidebar_activation_deps', '_make_thread_action_deps', '_make_thread_activation_deps', '_make_window_focus_deps', '_make_window_text_deps', '_print_background_watch_status', 'activate_thread_by_sidebar_v2', 'activate_thread_for_ipc', 'activate_thread_in_ui', 'annotations', 'cancel_codex_reply_if_busy', 'classify_permission_approval_ui_reply', 'click_window', 'ensure_codex_composer_focus', 'find_codex_window', 'focus_window', 'get_clipboard_text', 'get_window_text', 'is_codex_desktop_window_title', 'send_hotkey', 'send_key_event', 'send_prompt_to_codex', 'set_clipboard_text', 'start_background_watch', 'submit_permission_approval_via_ui', 'submit_permission_approval_via_ui_row_select', 'verify_active_thread', 'verify_active_thread_by_header', 'verify_thread_in_ui', 'wait_for_new_thread', 'wait_for_thread_activation', 'watch_for_final_answer')

from __future__ import annotations

from codex_desktop_bridge_impl_common import *

from typing import TYPE_CHECKING

import codex_discord_user_root_scope as discord_user_root_scope

if TYPE_CHECKING:
    from codex_desktop_bridge_impl_type_exports import find_codex_window


def detect_running_codex_desktop_executable() -> tuple[Path | None, str]:
    try:
        window = find_codex_window()
    except (window_focus.WindowFocusError, OSError, RuntimeError):
        return (None, "")
    pid = desktop_resolver.get_window_process_id(window.hwnd)
    if not pid:
        return (None, "")
    candidate = desktop_resolver.query_process_image_path(pid)
    if candidate is None:
        return (None, "")
    return (candidate, f"window-pid:{pid}")


def _sync_bridge_path_overrides() -> None:
    bridge_state.CODEX_HOME = CODEX_HOME
    bridge_state.GLOBAL_STATE_PATH = GLOBAL_STATE_PATH
    bridge_state.STATE_DB_PATH = STATE_DB_PATH
    bridge_state.SESSION_INDEX_PATH = SESSION_INDEX_PATH
    bridge_state.BRIDGE_STATE_PATH = BRIDGE_STATE_PATH
    bridge_state.LOG_DB_PATH = LOG_DB_PATH
    bridge_state.ARCHIVED_SESSIONS_DIR = ARCHIVED_SESSIONS_DIR
    bridge_state.MAINTENANCE_BACKUP_ROOT = MAINTENANCE_BACKUP_ROOT


rotate_single_backup_file = file_backup.rotate_single_backup_file
load_json = bridge_state.load_json
save_json = bridge_state.save_json
collapse_list_text = bridge_formatting.collapse_list_text
make_console_safe_text = bridge_formatting.make_console_safe_text
format_title_preview = bridge_formatting.format_title_preview
load_bridge_state = bridge_state.load_bridge_state
save_bridge_state = bridge_state.save_bridge_state
get_saved_thread_settings = bridge_state.get_saved_thread_settings
remember_thread_settings = bridge_state.remember_thread_settings
get_selected_thread_id = bridge_state.get_selected_thread_id
set_selected_thread_id = bridge_state.set_selected_thread_id
cache_live_approval_request = bridge_state.cache_live_approval_request
get_cached_live_approval_request = bridge_state.get_cached_live_approval_request
clear_cached_live_approval_request = bridge_state.clear_cached_live_approval_request
is_protocol_registered = bridge_protocol.is_protocol_registered
get_active_workspace_roots = session_index.get_active_workspace_roots
strip_windows_extended_prefix = session_index.strip_windows_extended_prefix
normalize_workspace_path = session_index.normalize_workspace_path
load_session_thread_names = session_index.load_session_thread_names
format_session_index_timestamp = session_index.format_session_index_timestamp
write_session_index_entries = session_index.write_session_index_entries
sync_session_index_with_state = session_sync.sync_session_index_with_state
normalize_ui_match_text = session_index.normalize_ui_match_text
build_ui_name_prefixes = session_index.build_ui_name_prefixes
get_thread_ui_name_candidates = session_index.get_thread_ui_name_candidates


def get_thread_ui_name(thread_id: str, thread: ThreadInfo | None = None) -> str | None:
    _sync_bridge_path_overrides()
    return thread_store.get_thread_ui_name(thread_id, thread)


def load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
    _sync_bridge_path_overrides()
    return thread_store.load_recent_threads(limit)


def load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
    _sync_bridge_path_overrides()
    return thread_store.load_user_root_threads(limit)


def load_ordinary_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
    _sync_bridge_path_overrides()
    return discord_user_root_scope.load_ordinary_user_root_threads(
        load_user_root_threads,
        limit=limit,
    )


def load_archived_threads(limit: int = 20) -> list[ThreadInfo]:
    _sync_bridge_path_overrides()
    return thread_store.load_archived_threads(limit)


def get_thread_by_id(thread_id: str, threads: list[ThreadInfo] | None = None) -> ThreadInfo:
    _sync_bridge_path_overrides()
    return thread_store.get_thread_by_id(thread_id, threads)


get_thread_workspace_name = thread_store.get_thread_workspace_name
get_thread_label = thread_store.get_thread_label
build_workspace_ref_map = thread_store.build_workspace_ref_map


def get_thread_workspace_ref(thread: ThreadInfo, threads: list[ThreadInfo] | None = None) -> str:
    _sync_bridge_path_overrides()
    return thread_store.get_thread_workspace_ref(thread, threads)


def resolve_thread_ref(thread_ref: str, limit: int = 50) -> ThreadInfo:
    _sync_bridge_path_overrides()
    return thread_store.resolve_thread_ref(thread_ref, limit)


def resolve_archived_thread_ref(thread_ref: str, limit: int = 100) -> ThreadInfo:
    _sync_bridge_path_overrides()
    return thread_store.resolve_archived_thread_ref(thread_ref, limit)


def get_thread_slot(thread: ThreadInfo, limit: int = 9) -> int | None:
    _sync_bridge_path_overrides()
    return thread_store.get_thread_slot(thread, limit)


def choose_thread(thread_id: str | None, cwd: str | None) -> ThreadInfo:
    _sync_bridge_path_overrides()
    return thread_store.choose_thread(thread_id, cwd)


def _choose_thread_from_args(args: argparse.Namespace) -> ThreadInfo:
    return choose_thread(_arg_optional_text(args, "thread_id"), _arg_optional_text(args, "cwd"))


def _choose_or_resolve_thread_from_args(args: argparse.Namespace) -> ThreadInfo:
    thread_ref = _arg_optional_text(args, "thread_ref")
    if thread_ref:
        return resolve_thread_ref(thread_ref)
    return _choose_thread_from_args(args)


format_timestamp = bridge_formatting.format_timestamp
format_token_k = bridge_formatting.format_token_k


__all__ = (
    "_choose_or_resolve_thread_from_args",
    "_choose_thread_from_args",
    "_sync_bridge_path_overrides",
    "build_ui_name_prefixes",
    "build_workspace_ref_map",
    "cache_live_approval_request",
    "choose_thread",
    "clear_cached_live_approval_request",
    "collapse_list_text",
    "detect_running_codex_desktop_executable",
    "format_session_index_timestamp",
    "format_timestamp",
    "format_title_preview",
    "format_token_k",
    "get_active_workspace_roots",
    "get_cached_live_approval_request",
    "get_saved_thread_settings",
    "get_selected_thread_id",
    "get_thread_by_id",
    "get_thread_label",
    "get_thread_slot",
    "get_thread_ui_name",
    "get_thread_ui_name_candidates",
    "get_thread_workspace_name",
    "get_thread_workspace_ref",
    "is_protocol_registered",
    "load_archived_threads",
    "load_bridge_state",
    "load_json",
    "load_ordinary_user_root_threads",
    "load_recent_threads",
    "load_session_thread_names",
    "load_user_root_threads",
    "make_console_safe_text",
    "normalize_ui_match_text",
    "normalize_workspace_path",
    "remember_thread_settings",
    "resolve_archived_thread_ref",
    "resolve_thread_ref",
    "rotate_single_backup_file",
    "save_bridge_state",
    "save_json",
    "set_selected_thread_id",
    "strip_windows_extended_prefix",
    "sync_session_index_with_state",
    "write_session_index_entries",
)

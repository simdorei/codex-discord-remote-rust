from __future__ import annotations

from pathlib import Path

import codex_desktop_bridge_session_index as session_index
import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_store_db as thread_store_db
from codex_thread_models import ThreadInfo


class ThreadStoreError(RuntimeError):
    pass


class ThreadNotFoundError(ThreadStoreError):
    def __init__(self, thread_ref: str) -> None:
        self.thread_ref: str = thread_ref
        super().__init__(f"Thread not found: {thread_ref}")


class NoCodexThreadsError(ThreadStoreError):
    def __init__(self) -> None:
        super().__init__("No Codex threads found in the local state DB.")


class NoArchivedCodexThreadsError(ThreadStoreError):
    def __init__(self) -> None:
        super().__init__("No archived Codex threads found in the local state DB.")


class NoAlternateThreadError(ThreadStoreError):
    def __init__(self) -> None:
        super().__init__("No alternate thread found.")


class ThreadIndexOutOfRangeError(ThreadStoreError):
    def __init__(self, thread_ref: str) -> None:
        self.thread_ref: str = thread_ref
        super().__init__(f"Thread index out of range: {thread_ref}")


class ArchivedThreadIndexOutOfRangeError(ThreadStoreError):
    def __init__(self, thread_ref: str) -> None:
        self.thread_ref: str = thread_ref
        super().__init__(f"Archived thread index out of range: {thread_ref}")


class AmbiguousThreadRefError(ThreadStoreError):
    def __init__(self, thread_ref: str, refs: str) -> None:
        self.thread_ref: str = thread_ref
        self.refs: str = refs
        super().__init__(f"Multiple threads match workspace `{thread_ref}`. Use one of: {refs}")


class AmbiguousArchivedThreadRefError(ThreadStoreError):
    def __init__(self, thread_ref: str, refs: str) -> None:
        self.thread_ref: str = thread_ref
        self.refs: str = refs
        super().__init__(f"Multiple archived threads match workspace `{thread_ref}`. Use one of: {refs}")


def load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
    return thread_store_db.load_recent_threads(limit)


def load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
    return thread_store_db.load_user_root_threads(limit)


def load_archived_threads(limit: int = 20) -> list[ThreadInfo]:
    return thread_store_db.load_archived_threads(limit)


def get_thread_ui_name(thread_id: str, thread: ThreadInfo | None = None) -> str | None:
    if thread is None:
        try:
            thread = get_thread_by_id(thread_id)
        except RuntimeError:
            session_name = session_index.normalize_ui_match_text(
                session_index.load_session_thread_names().get(thread_id, "")
            )
            return session_name or None

    candidates = session_index.get_thread_ui_name_candidates(thread)
    return candidates[0] if candidates else None


def get_thread_by_id(thread_id: str, threads: list[ThreadInfo] | None = None) -> ThreadInfo:
    pool = threads or load_recent_threads(limit=50)
    for thread in pool:
        if thread.id == thread_id:
            return thread
    raise ThreadNotFoundError(thread_id)


def get_thread_workspace_name(thread: ThreadInfo) -> str:
    cwd = session_index.strip_windows_extended_prefix((thread.cwd or "").strip())
    if not cwd:
        return "-"
    return Path(cwd).name or cwd


def get_thread_label(thread: ThreadInfo) -> str:
    return f"{get_thread_workspace_name(thread)}:{thread.id[:8]}"


def build_workspace_ref_map(threads: list[ThreadInfo]) -> dict[str, str]:
    totals: dict[str, int] = {}
    for thread in threads:
        workspace = get_thread_workspace_name(thread)
        totals[workspace] = totals.get(workspace, 0) + 1

    seen: dict[str, int] = {}
    mapping: dict[str, str] = {}
    for thread in threads:
        workspace = get_thread_workspace_name(thread)
        seen[workspace] = seen.get(workspace, 0) + 1
        if totals.get(workspace, 0) > 1:
            mapping[thread.id] = f"{workspace}:{seen[workspace]}"
        else:
            mapping[thread.id] = workspace
    return mapping


def get_thread_workspace_ref(thread: ThreadInfo, threads: list[ThreadInfo] | None = None) -> str:
    pool = threads or load_recent_threads(limit=50)
    return build_workspace_ref_map(pool).get(thread.id, get_thread_workspace_name(thread))


def resolve_thread_ref(thread_ref: str, limit: int = 50) -> ThreadInfo:
    threads = load_recent_threads(limit=limit)
    if not threads:
        raise NoCodexThreadsError()

    normalized = thread_ref.strip().lower()
    if normalized in {"other", "next"}:
        selected_thread_id = bridge_state.get_selected_thread_id()
        for thread in threads:
            if thread.id != selected_thread_id:
                return thread
        raise NoAlternateThreadError()

    if thread_ref.isdigit():
        index = int(thread_ref)
        if 1 <= index <= len(threads):
            return threads[index - 1]
        raise ThreadIndexOutOfRangeError(thread_ref)

    workspace_map = build_workspace_ref_map(threads)
    for thread in threads:
        if workspace_map.get(thread.id, "").lower() == normalized:
            return thread

    for thread in threads:
        if session_index.normalize_workspace_path(thread.cwd) == session_index.normalize_workspace_path(thread_ref):
            return thread

    workspace_matches = [thread for thread in threads if get_thread_workspace_name(thread).lower() == normalized]
    if len(workspace_matches) > 1:
        refs = ", ".join(workspace_map.get(thread.id, thread.id) for thread in workspace_matches)
        raise AmbiguousThreadRefError(thread_ref, refs)
    for thread in threads:
        if get_thread_workspace_name(thread).lower() == normalized:
            return thread

    return get_thread_by_id(thread_ref, threads=load_recent_threads(limit=0))


def resolve_archived_thread_ref(thread_ref: str, limit: int = 100) -> ThreadInfo:
    threads = load_archived_threads(limit=limit)
    if not threads:
        raise NoArchivedCodexThreadsError()

    normalized = thread_ref.strip().lower()

    if thread_ref.isdigit():
        index = int(thread_ref)
        if 1 <= index <= len(threads):
            return threads[index - 1]
        raise ArchivedThreadIndexOutOfRangeError(thread_ref)

    workspace_map = build_workspace_ref_map(threads)
    for thread in threads:
        if workspace_map.get(thread.id, "").lower() == normalized:
            return thread

    for thread in threads:
        if session_index.normalize_workspace_path(thread.cwd) == session_index.normalize_workspace_path(thread_ref):
            return thread

    workspace_matches = [thread for thread in threads if get_thread_workspace_name(thread).lower() == normalized]
    if len(workspace_matches) > 1:
        refs = ", ".join(workspace_map.get(thread.id, thread.id) for thread in workspace_matches)
        raise AmbiguousArchivedThreadRefError(thread_ref, refs)
    for thread in threads:
        if get_thread_workspace_name(thread).lower() == normalized:
            return thread

    return get_thread_by_id(thread_ref, threads=load_archived_threads(limit=0))


def get_thread_slot(thread: ThreadInfo, limit: int = 9) -> int | None:
    threads = load_recent_threads(limit=max(limit, 9))
    for index, item in enumerate(threads, start=1):
        if item.id == thread.id:
            return index
    return None


def choose_thread(thread_id: str | None, cwd: str | None) -> ThreadInfo:
    threads = load_recent_threads(limit=50)
    if not threads:
        raise NoCodexThreadsError()

    if thread_id:
        for thread in threads:
            if thread.id == thread_id:
                return thread
        return get_thread_by_id(thread_id, threads=load_recent_threads(limit=0))

    if cwd:
        target = session_index.normalize_workspace_path(cwd)
        for thread in threads:
            if session_index.normalize_workspace_path(thread.cwd) == target:
                return thread

    selected_thread_id = bridge_state.get_selected_thread_id()
    if selected_thread_id:
        for thread in threads:
            if thread.id == selected_thread_id:
                return thread

    active_roots = session_index.get_active_workspace_roots()
    if active_roots:
        active_set = {session_index.normalize_workspace_path(root) for root in active_roots}
        for thread in threads:
            if session_index.normalize_workspace_path(thread.cwd) in active_set:
                return thread

    return threads[0]

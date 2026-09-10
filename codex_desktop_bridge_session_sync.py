from __future__ import annotations

import time

import codex_desktop_bridge_session_files as session_file_threads
import codex_desktop_bridge_session_index as session_index
import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_store as thread_store
from codex_bridge_state import JsonObject


def sync_session_index_with_state() -> int:
    threads = thread_store.load_recent_threads(limit=0)
    existing_ids = {thread.id for thread in threads}
    threads.extend(
        session_file_threads.load_missing_vscode_rollout_threads(
            bridge_state.CODEX_HOME / "sessions",
            existing_ids,
            session_thread_names=session_index.load_session_thread_names(),
        )
    )
    threads.sort(key=lambda thread: thread.updated_at, reverse=True)
    entries: list[JsonObject] = [
        {
            "id": thread.id,
            "thread_name": thread.title or thread_store.get_thread_ui_name(thread.id, thread) or thread.id,
            "updated_at": session_index.format_session_index_timestamp(float(thread.updated_at or time.time())),
        }
        for thread in threads
    ]
    session_index.write_session_index_entries(entries)
    return len(entries)

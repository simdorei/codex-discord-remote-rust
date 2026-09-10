from __future__ import annotations

from typing import TypedDict

import codex_discord_bridge_protocols as discord_bridge_protocols


class RefreshBridgeSessionState(TypedDict):
    session_index_count: int
    thread_count: int
    selected_before: str
    selected_thread_id: str
    selected_ref: str
    selected_action: str


def refresh_codex_bridge_session_state(
    bridge_session_state: discord_bridge_protocols.CodexBridgeSessionStateModule,
) -> RefreshBridgeSessionState:
    session_index_count = bridge_session_state.sync_session_index_with_state()
    threads = bridge_session_state.load_recent_threads(limit=0)
    selected_before = bridge_session_state.get_selected_thread_id()
    selected_thread = next((thread for thread in threads if thread.id == selected_before), None)
    if selected_thread is not None:
        selected_action = "kept"
    elif threads:
        selected_thread = bridge_session_state.choose_thread(None, None)
        bridge_session_state.set_selected_thread_id(selected_thread.id)
        selected_action = "initialized" if not selected_before else "stale_replaced"
    else:
        if selected_before:
            bridge_session_state.set_selected_thread_id(None)
        selected_action = "cleared" if selected_before else "none"

    selected_ref = ""
    if selected_thread is not None:
        try:
            selected_ref = bridge_session_state.get_thread_workspace_ref(selected_thread, threads)
        except (OSError, RuntimeError):
            selected_ref = bridge_session_state.get_thread_workspace_name(selected_thread)
    return {
        "session_index_count": session_index_count,
        "thread_count": len(threads),
        "selected_before": selected_before or "-",
        "selected_thread_id": selected_thread.id if selected_thread else "-",
        "selected_ref": selected_ref or "-",
        "selected_action": selected_action,
    }

from __future__ import annotations

import json
from collections.abc import Callable

import codex_desktop_bridge_state as bridge_state
from codex_bridge_state import JsonValue


_decode_json_value: Callable[[str], JsonValue] = json.loads


def scrub_bridge_state_deleted_thread(thread_id: str) -> list[str]:
    data = bridge_state.load_bridge_state()
    changed: list[str] = []
    if data.get("selected_thread_id") == thread_id:
        _ = data.pop("selected_thread_id", None)
        changed.append("selected_thread_id")
    recent_live_approval_requests = data.get("recent_live_approval_requests")
    if isinstance(recent_live_approval_requests, dict) and thread_id in recent_live_approval_requests:
        _ = recent_live_approval_requests.pop(thread_id, None)
        if recent_live_approval_requests:
            data["recent_live_approval_requests"] = recent_live_approval_requests
        else:
            _ = data.pop("recent_live_approval_requests", None)
        changed.append("recent_live_approval_requests")
    recent_ui_thread = data.get("recent_ui_thread")
    if isinstance(recent_ui_thread, dict) and str(recent_ui_thread.get("thread_id") or "") == thread_id:
        _ = data.pop("recent_ui_thread", None)
        changed.append("recent_ui_thread")
    if changed:
        bridge_state.save_bridge_state(data)
    return changed


def scrub_global_state_deleted_thread(thread_id: str) -> list[str]:
    if not bridge_state.GLOBAL_STATE_PATH.exists():
        return []
    data = bridge_state.load_json(bridge_state.GLOBAL_STATE_PATH)
    changed: list[str] = []
    queued_follow_ups = data.get("queued-follow-ups")
    if isinstance(queued_follow_ups, dict) and thread_id in queued_follow_ups:
        _ = queued_follow_ups.pop(thread_id, None)
        changed.append("queued-follow-ups")
    pinned_thread_ids = data.get("pinned-thread-ids")
    if isinstance(pinned_thread_ids, list):
        filtered = [item for item in pinned_thread_ids if str(item) != thread_id]
        if len(filtered) != len(pinned_thread_ids):
            data["pinned-thread-ids"] = filtered
            changed.append("pinned-thread-ids")
    if changed:
        bridge_state.save_json(bridge_state.GLOBAL_STATE_PATH, data)
    return changed


def scrub_session_index_deleted_thread(thread_id: str) -> int:
    if not bridge_state.SESSION_INDEX_PATH.exists():
        return 0
    original = bridge_state.SESSION_INDEX_PATH.read_text(encoding="utf-8")
    kept_lines: list[str] = []
    removed = 0
    for raw_line in original.splitlines():
        line = raw_line.strip()
        if not line:
            kept_lines.append(raw_line)
            continue
        try:
            payload = _decode_json_value(line)
        except json.JSONDecodeError:
            kept_lines.append(raw_line)
            continue
        if isinstance(payload, dict) and str(payload.get("id") or "") == thread_id:
            removed += 1
            continue
        kept_lines.append(raw_line)
    if removed:
        rewritten = "\n".join(kept_lines)
        if original.endswith(("\n", "\r")) and rewritten:
            rewritten += "\n"
        _ = bridge_state.SESSION_INDEX_PATH.write_text(rewritten, encoding="utf-8")
    return removed

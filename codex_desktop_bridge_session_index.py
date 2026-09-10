from __future__ import annotations

import json
import os
import time
from collections.abc import Callable
from datetime import datetime, timezone
from pathlib import Path

import codex_desktop_bridge_state as bridge_state
from codex_bridge_state import JsonObject, JsonValue
from codex_thread_models import ThreadInfo


_decode_json_value: Callable[[str], JsonValue] = json.loads


def get_active_workspace_roots() -> list[str]:
    data = bridge_state.load_json(bridge_state.GLOBAL_STATE_PATH)
    roots_value = data.get("active-workspace-roots")
    if not isinstance(roots_value, list):
        return []
    return [str(Path(root)) for root in roots_value if isinstance(root, str)]


def strip_windows_extended_prefix(path: str) -> str:
    value = str(path or "").strip()
    if value.startswith("\\\\?\\UNC\\"):
        return "\\\\" + value[8:]
    if value.startswith("\\\\?\\"):
        return value[4:]
    return value


def normalize_workspace_path(path: str) -> str:
    value = strip_windows_extended_prefix(path)
    if not value:
        return ""
    return os.path.normcase(os.path.normpath(value))


def load_session_thread_names() -> dict[str, str]:
    mapping: dict[str, str] = {}
    if not bridge_state.SESSION_INDEX_PATH.exists():
        return mapping

    for raw_line in bridge_state.SESSION_INDEX_PATH.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        try:
            payload_value = _decode_json_value(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(payload_value, dict):
            continue

        thread_id = payload_value.get("id")
        thread_name = payload_value.get("thread_name")
        if isinstance(thread_id, str) and isinstance(thread_name, str) and thread_name.strip():
            mapping[thread_id] = thread_name.strip()
    return mapping


def format_session_index_timestamp(ts: float | None = None) -> str:
    moment = datetime.fromtimestamp(ts or time.time(), tz=timezone.utc)
    return moment.strftime("%Y-%m-%dT%H:%M:%S.") + f"{moment.microsecond:06d}0Z"


def write_session_index_entries(entries: list[JsonObject]) -> None:
    bridge_state.SESSION_INDEX_PATH.parent.mkdir(parents=True, exist_ok=True)
    rendered = "\n".join(json.dumps(item, ensure_ascii=False) for item in entries)
    if rendered:
        rendered += "\n"
    _ = bridge_state.SESSION_INDEX_PATH.write_text(rendered, encoding="utf-8")


def normalize_ui_match_text(text: str) -> str:
    raw = str(text or "").replace("\r", "\n")
    for line in raw.split("\n"):
        normalized = " ".join(line.split()).strip()
        if normalized:
            return normalized
    return ""


def build_ui_name_prefixes(text: str) -> list[str]:
    text = normalize_ui_match_text(text)
    if not text:
        return []

    candidates = [text]
    for limit in (120, 96, 72, 56, 40):
        if len(text) > limit:
            candidate = text[:limit].rstrip(" .,;:!?-")
            if candidate:
                candidates.append(candidate)

    deduped: list[str] = []
    seen: set[str] = set()
    for candidate in candidates:
        lowered = candidate.lower()
        if lowered in seen:
            continue
        seen.add(lowered)
        deduped.append(candidate)
    return deduped


def get_thread_ui_name_candidates(thread: ThreadInfo) -> list[str]:
    candidates: list[str] = []

    session_name = normalize_ui_match_text(load_session_thread_names().get(thread.id, ""))
    if session_name:
        candidates.append(session_name)

    title_name = normalize_ui_match_text(thread.title)
    if title_name:
        candidates.extend(build_ui_name_prefixes(title_name))

    deduped: list[str] = []
    seen: set[str] = set()
    for candidate in candidates:
        lowered = candidate.lower()
        if lowered in seen:
            continue
        seen.add(lowered)
        deduped.append(candidate)
    return deduped

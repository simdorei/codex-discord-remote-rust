from __future__ import annotations

import json
import re
from collections.abc import Callable
from collections.abc import Mapping
from pathlib import Path
from typing import Final

from codex_bridge_state import JsonObject, JsonValue
from codex_thread_models import ThreadInfo

SESSION_ID_RE: Final = re.compile(
    r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$"
)
_decode_json_value: Callable[[str], JsonValue] = json.loads


class _RolloutParseState:
    source: str
    thread_source: str
    cwd: str
    title: str
    model: str
    reasoning_effort: str

    __slots__: tuple[str, ...] = ("source", "thread_source", "cwd", "title", "model", "reasoning_effort")

    def __init__(
        self,
        *,
        source: str = "",
        thread_source: str = "",
        cwd: str = "",
        title: str = "",
        model: str = "",
        reasoning_effort: str = "",
    ) -> None:
        self.source = source
        self.thread_source = thread_source
        self.cwd = cwd
        self.title = title
        self.model = model
        self.reasoning_effort = reasoning_effort


def _first_line(text: str) -> str:
    for line in str(text or "").replace("\r", "\n").split("\n"):
        value = " ".join(line.split()).strip()
        if value:
            return value
    return ""


def _session_id_from_path(path: Path) -> str:
    match = SESSION_ID_RE.search(path.name)
    return match.group(1) if match else ""


def _string_payload_value(payload: Mapping[str, JsonValue], key: str, fallback: str) -> str:
    value = payload.get(key)
    return value if isinstance(value, str) and value else fallback


def _rollout_state(thread_id: str, session_thread_names: Mapping[str, str] | None) -> _RolloutParseState:
    return _RolloutParseState(title=_first_line((session_thread_names or {}).get(thread_id, "")))


def _payload_from_rollout_line(raw_line: str) -> tuple[str, JsonObject] | None:
    line = raw_line.strip()
    if not line:
        return None
    try:
        event_value = _decode_json_value(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(event_value, dict):
        return None
    event: JsonObject = event_value
    payload = event.get("payload")
    if not isinstance(payload, dict):
        return None
    return _string_payload_value(event, "type", ""), payload


def _apply_session_meta(state: _RolloutParseState, payload: Mapping[str, JsonValue]) -> None:
    state.source = _string_payload_value(payload, "source", state.source)
    state.thread_source = _string_payload_value(payload, "thread_source", state.thread_source)
    state.cwd = _string_payload_value(payload, "cwd", state.cwd)


def _apply_turn_context(state: _RolloutParseState, payload: JsonObject) -> None:
    state.cwd = _string_payload_value(payload, "cwd", state.cwd)
    state.model = _string_payload_value(payload, "model", state.model)
    state.reasoning_effort = _string_payload_value(payload, "reasoning_effort", state.reasoning_effort)
    collaboration_mode_value = payload.get("collaboration_mode")
    if isinstance(collaboration_mode_value, dict):
        settings_value = collaboration_mode_value.get("settings")
        if isinstance(settings_value, dict):
            _apply_turn_context_settings(state, settings_value)


def _apply_turn_context_settings(state: _RolloutParseState, settings: Mapping[str, JsonValue]) -> None:
    state.model = _string_payload_value(settings, "model", state.model)
    state.reasoning_effort = _string_payload_value(settings, "reasoning_effort", state.reasoning_effort)


def _apply_rollout_payload(state: _RolloutParseState, event_type: str, payload: JsonObject) -> None:
    if event_type == "session_meta":
        _apply_session_meta(state, payload)
    elif event_type == "turn_context":
        _apply_turn_context(state, payload)
    elif _string_payload_value(payload, "type", "") == "user_message" and not state.title:
        state.title = _first_line(_string_payload_value(payload, "message", ""))


def _has_rollout_thread_summary(state: _RolloutParseState) -> bool:
    return bool(state.source and state.cwd and state.title and state.model)


def _is_loadable_vscode_rollout(state: _RolloutParseState) -> bool:
    return (
        state.source == "vscode"
        and (not state.thread_source or state.thread_source == "user")
        and bool(state.cwd)
        and bool(state.title)
    )


def _thread_info_from_state(path: Path, thread_id: str, state: _RolloutParseState) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=state.title,
        cwd=state.cwd,
        updated_at=int(path.stat().st_mtime),
        rollout_path=str(path),
        model=state.model,
        reasoning_effort=state.reasoning_effort,
        tokens_used=0,
    )


def _thread_from_rollout_path(
    path: Path,
    thread_id: str,
    *,
    session_thread_names: Mapping[str, str] | None = None,
) -> ThreadInfo | None:
    state = _rollout_state(thread_id, session_thread_names)
    with path.open(encoding="utf-8") as handle:
        for raw_line in handle:
            event_payload = _payload_from_rollout_line(raw_line)
            if event_payload is None:
                continue
            event_type, payload = event_payload
            _apply_rollout_payload(state, event_type, payload)
            if _has_rollout_thread_summary(state):
                break
    if not _is_loadable_vscode_rollout(state):
        return None
    return _thread_info_from_state(path, thread_id, state)


def load_missing_vscode_rollout_threads(
    sessions_dir: Path,
    existing_thread_ids: set[str],
    *,
    session_thread_names: Mapping[str, str] | None = None,
) -> list[ThreadInfo]:
    if not sessions_dir.exists():
        return []
    threads: list[ThreadInfo] = []
    for path in sessions_dir.rglob("rollout-*.jsonl"):
        thread_id = _session_id_from_path(path)
        if not thread_id or thread_id in existing_thread_ids:
            continue
        thread = _thread_from_rollout_path(
            path,
            thread_id,
            session_thread_names=session_thread_names,
        )
        if thread is not None:
            threads.append(thread)
    return threads

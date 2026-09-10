from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path

from codex_bridge_state import JsonObject, JsonValue


_decode_json_value: Callable[[str], JsonValue] = json.loads


def read_new_session_events(
    session_path: Path,
    start_offset: int,
    *,
    max_events: int | None = None,
) -> tuple[list[JsonObject], int]:
    events: list[JsonObject] = []
    if not session_path.exists():
        return events, start_offset

    event_limit = max(0, int(max_events or 0))
    with session_path.open("r", encoding="utf-8") as handle:
        _ = handle.seek(start_offset)
        while True:
            pos = handle.tell()
            raw = handle.readline()
            if not raw:
                return events, pos

            line = raw.strip()
            if not line:
                continue

            try:
                payload = _decode_json_value(line)
            except json.JSONDecodeError:
                _ = handle.seek(pos)
                return events, pos
            if isinstance(payload, dict):
                events.append(payload)
            if event_limit and len(events) >= event_limit:
                return events, handle.tell()

from __future__ import annotations

from collections.abc import Callable, Iterator
import json
from pathlib import Path
from typing import TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonEvent: TypeAlias = dict[str, JsonValue]

_decode_json_value: Callable[[str], JsonValue] = json.loads


def iter_session_events(session_path: Path) -> Iterator[JsonEvent]:
    with session_path.open("r", encoding="utf-8") as handle:
        for raw in handle:
            line = raw.strip()
            if not line:
                continue
            try:
                event = _decode_json_value(line)
            except json.JSONDecodeError:
                continue
            if isinstance(event, dict):
                yield event

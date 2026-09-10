from __future__ import annotations

from collections.abc import Mapping


QueueRetractValue = int | bool | str | None


def _coerce_int(value: QueueRetractValue) -> int:
    if isinstance(value, bool) or value is None:
        return int(bool(value))
    if isinstance(value, int):
        return value
    try:
        return int(value)
    except ValueError:
        return 0


def build_retract_message(result: Mapping[str, QueueRetractValue], target_ref: str) -> str:
    removed = _coerce_int(result.get("removed"))
    remaining = _coerce_int(result.get("remaining"))
    active = bool(result.get("active"))
    if removed:
        return f"Retracted your latest queued ask for `{target_ref}`. remaining_queued: {remaining}"
    if active:
        return f"No queued ask from you for `{target_ref}`. The active ask cannot be retracted from Discord."
    return f"No queued ask from you for `{target_ref}`."

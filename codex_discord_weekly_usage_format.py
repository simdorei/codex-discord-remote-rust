from __future__ import annotations

from collections.abc import Callable
from datetime import datetime, timezone
from typing import Protocol

from codex_session_events import JsonValue


class WeeklyUsageFormatBridge(Protocol):
    def coerce_nonnegative_int(self, value: JsonValue | None) -> int: ...
    def format_timestamp(self, unix_seconds: int) -> str: ...


FormatPercentFunc = Callable[[JsonValue | None], str]


def parse_event_timestamp(value: JsonValue | None) -> datetime | None:
    if not isinstance(value, str) or not value.strip():
        return None
    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        moment = datetime.fromisoformat(text)
    except ValueError:
        return None
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=timezone.utc)
    return moment.astimezone(timezone.utc)


def format_window_minutes(value: JsonValue | None, *, bridge_module: WeeklyUsageFormatBridge) -> str:
    minutes = bridge_module.coerce_nonnegative_int(value)
    if minutes <= 0:
        return "-"
    if minutes % 1440 == 0:
        return f"{minutes // 1440}d"
    if minutes % 60 == 0:
        return f"{minutes // 60}h"
    return f"{minutes}m"


def format_rate_limit_reset(value: JsonValue | None, *, bridge_module: WeeklyUsageFormatBridge) -> str:
    reset_at = bridge_module.coerce_nonnegative_int(value)
    if reset_at <= 0:
        return "-"
    return bridge_module.format_timestamp(reset_at)


def format_rate_limit_line(
    label: str,
    value: JsonValue | None,
    *,
    bridge_module: WeeklyUsageFormatBridge,
    format_percent_func: FormatPercentFunc,
) -> str:
    if not isinstance(value, dict):
        return f"{label}: -"
    return (
        f"{label}: used={format_percent_func(value.get('used_percent'))} "
        f"window={format_window_minutes(value.get('window_minutes'), bridge_module=bridge_module)} "
        f"resets={format_rate_limit_reset(value.get('resets_at'), bridge_module=bridge_module)}"
    )

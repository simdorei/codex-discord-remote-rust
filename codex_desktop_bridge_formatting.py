from __future__ import annotations

import time


def collapse_list_text(value: str, limit: int = 70) -> str:
    collapsed = " ".join((value or "").replace("\r", " ").replace("\n", " ").split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: max(0, limit - 3)].rstrip() + "..."


def make_console_safe_text(value: str | None) -> str:
    return "" if value is None else str(value)


def format_title_preview(value: str, limit: int = 120) -> str:
    return make_console_safe_text(collapse_list_text(value, limit=limit))


def format_timestamp(unix_seconds: int) -> str:
    if not unix_seconds:
        return "-"
    return time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(unix_seconds))


def format_token_k(value: int) -> str:
    if value <= 0:
        return "-"
    if value < 1000:
        return str(value)
    if value >= 1_000_000:
        return f"{value / 1_000_000:.1f}M"
    return f"{value / 1000:.1f}k"

from __future__ import annotations

from typing import Protocol

from codex_discord_context_status import (
    ContextStatusBridge,
    GetMirroredThreadFunc,
    LogFunc,
    ResolveSelectedTargetFunc,
    ResolveTargetRefFunc,
    build_context_message,
    build_context_warning,
    format_context_usage_line,
)
from codex_discord_weekly_usage import (
    FormatPercentFunc,
    WeeklyUsageBridge,
    build_weekly_usage_message,
    format_rate_limit_line,
    format_rate_limit_reset,
    format_window_minutes,
    parse_event_timestamp,
)

__all__ = [
    "DiscordContextBridge",
    "FormatPercentFunc",
    "GetMirroredThreadFunc",
    "LogFunc",
    "ResolveSelectedTargetFunc",
    "ResolveTargetRefFunc",
    "WeeklyUsageBridge",
    "build_context_message",
    "build_context_warning",
    "build_weekly_usage_message",
    "format_context_usage_line",
    "format_rate_limit_line",
    "format_rate_limit_reset",
    "format_window_minutes",
    "parse_event_timestamp",
]


class DiscordContextBridge(ContextStatusBridge, WeeklyUsageBridge, Protocol):
    pass

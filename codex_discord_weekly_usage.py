from __future__ import annotations

from collections.abc import Callable
from datetime import date, datetime, timedelta, timezone
from typing import Protocol

from codex_app_server_transport import PersistentCodexAppServer
from codex_app_server_transport_replies import JsonMapping, JsonObject, JsonValue
from codex_discord_weekly_usage_format import (
    FormatPercentFunc,
    format_rate_limit_line,
    format_rate_limit_reset,
    format_window_minutes,
    parse_event_timestamp,
)
from codex_discord_weekly_usage_scan import (
    WeeklyUsageBridge,
    WeeklyUsageScanResult,
    scan_weekly_usage_events,
)

__all__ = [
    "FormatPercentFunc",
    "WeeklyUsageBridge",
    "WeeklyUsageScanResult",
    "build_weekly_usage_message",
    "format_rate_limit_line",
    "format_rate_limit_reset",
    "format_window_minutes",
    "parse_event_timestamp",
    "scan_weekly_usage_events",
]

LIVE_USAGE_TIMEOUT_SEC = 15.0


class UsageTransport(Protocol):
    def request(
        self,
        method: str,
        params: JsonMapping | None = None,
        *,
        timeout_sec: float = 10.0,
    ) -> JsonObject: ...

    def close(self) -> None: ...


UsageTransportFactory = Callable[[], UsageTransport]
NowFunc = Callable[[], datetime]


def _as_object(value: JsonValue | None) -> JsonObject | None:
    return value if isinstance(value, dict) else None


def _normalize_rate_window(value: JsonValue | None) -> JsonObject | None:
    window = _as_object(value)
    if window is None:
        return None
    return {
        "used_percent": window.get("usedPercent"),
        "window_minutes": window.get("windowDurationMins"),
        "resets_at": window.get("resetsAt"),
    }


def _format_credits(value: JsonValue | None) -> str:
    credits = _as_object(value)
    if credits is None:
        return "credits: -"
    has_credits = "yes" if credits.get("hasCredits") is True else "no"
    unlimited = "yes" if credits.get("unlimited") is True else "no"
    return (
        f"credits: balance={credits.get('balance') or '0'} "
        f"has_credits={has_credits} unlimited={unlimited}"
    )


def _parse_bucket_date(value: JsonValue | None) -> date | None:
    if not isinstance(value, str):
        return None
    try:
        return date.fromisoformat(value)
    except ValueError:
        return None


def _daily_usage_rows(
    usage: JsonObject,
    *,
    first_day: date,
    last_day: date,
    bridge_module: WeeklyUsageBridge,
) -> list[tuple[date, int]]:
    buckets = usage.get("dailyUsageBuckets")
    if not isinstance(buckets, list):
        return []
    rows: list[tuple[date, int]] = []
    for bucket_value in buckets:
        bucket = _as_object(bucket_value)
        if bucket is None:
            continue
        bucket_day = _parse_bucket_date(bucket.get("startDate"))
        if bucket_day is None or not first_day <= bucket_day <= last_day:
            continue
        tokens = bridge_module.coerce_nonnegative_int(bucket.get("tokens"))
        rows.append((bucket_day, tokens))
    return sorted(rows)


def _query_live_usage(
    transport_factory: UsageTransportFactory,
) -> tuple[JsonObject, JsonObject]:
    transport = transport_factory()
    try:
        rate_limits = transport.request(
            "account/rateLimits/read",
            {},
            timeout_sec=LIVE_USAGE_TIMEOUT_SEC,
        )
        usage = transport.request(
            "account/usage/read",
            {},
            timeout_sec=LIVE_USAGE_TIMEOUT_SEC,
        )
        return rate_limits, usage
    finally:
        transport.close()


def _format_summary(
    summary: JsonObject | None,
    *,
    bridge_module: WeeklyUsageBridge,
) -> list[str]:
    if summary is None:
        return ["Account summary", "unavailable"]
    return [
        "Account summary",
        f"current_streak_days: {bridge_module.coerce_nonnegative_int(summary.get('currentStreakDays'))}",
        f"longest_streak_days: {bridge_module.coerce_nonnegative_int(summary.get('longestStreakDays'))}",
        f"lifetime_tokens: {bridge_module.format_token_k(bridge_module.coerce_nonnegative_int(summary.get('lifetimeTokens')))}",
        f"peak_daily_tokens: {bridge_module.format_token_k(bridge_module.coerce_nonnegative_int(summary.get('peakDailyTokens')))}",
        f"longest_running_turn_sec: {bridge_module.coerce_nonnegative_int(summary.get('longestRunningTurnSec'))}",
    ]


def build_weekly_usage_message(
    days: int = 7,
    *,
    bridge_module: WeeklyUsageBridge,
    format_percent_func: FormatPercentFunc,
    transport_factory: UsageTransportFactory = PersistentCodexAppServer,
    now_func: NowFunc = lambda: datetime.now(timezone.utc),
) -> str:
    days = max(1, min(30, days))
    queried_at = now_func().astimezone(timezone.utc)
    try:
        rate_result, usage = _query_live_usage(transport_factory)
    except Exception as exc:  # noqa: BLE001 - command output must expose live query failures.
        return f"Live usage unavailable: {type(exc).__name__}: {exc}"

    rate_limits = _as_object(rate_result.get("rateLimits")) or {}
    last_day = queried_at.date()
    first_day = last_day - timedelta(days=days - 1)
    rows = _daily_usage_rows(
        usage,
        first_day=first_day,
        last_day=last_day,
        bridge_module=bridge_module,
    )
    total_tokens = sum(tokens for _, tokens in rows)
    lines = [
        f"Codex usage ({days}d live)",
        f"queried_at: {queried_at.strftime('%Y-%m-%d %H:%M:%S UTC')}",
        "",
        "Live rate limits",
        f"plan: {rate_limits.get('planType') or '-'}",
        f"limit_id: {rate_limits.get('limitId') or '-'}",
        format_rate_limit_line(
            "primary",
            _normalize_rate_window(rate_limits.get("primary")),
            bridge_module=bridge_module,
            format_percent_func=format_percent_func,
        ),
        format_rate_limit_line(
            "secondary",
            _normalize_rate_window(rate_limits.get("secondary")),
            bridge_module=bridge_module,
            format_percent_func=format_percent_func,
        ),
        _format_credits(rate_limits.get("credits")),
        f"reached: {rate_limits.get('rateLimitReachedType') or '-'}",
        "",
        f"Daily token usage ({first_day.isoformat()} to {last_day.isoformat()})",
    ]
    lines.extend(
        f"{bucket_day.isoformat()}: {bridge_module.format_token_k(tokens)}"
        for bucket_day, tokens in rows
    )
    if not rows:
        lines.append("no usage buckets returned for this period")
    lines.extend(
        [
            f"total_tokens: {bridge_module.format_token_k(total_tokens)}",
            "",
            *_format_summary(
                _as_object(usage.get("summary")),
                bridge_module=bridge_module,
            ),
        ]
    )
    return "\n".join(lines)

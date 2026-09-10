from __future__ import annotations

from collections.abc import Iterator, Mapping
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from types import MappingProxyType
from typing import Protocol

from codex_discord_weekly_usage_format import parse_event_timestamp
from codex_session_events import JsonEvent, JsonValue

__all__ = [
    "WeeklyUsageBridge",
    "WeeklyUsageScanResult",
    "scan_weekly_usage_events",
]


class WeeklyUsageBridge(Protocol):
    CODEX_HOME: Path

    def coerce_nonnegative_int(self, value: JsonValue | None) -> int: ...
    def format_timestamp(self, unix_seconds: int) -> str: ...
    def format_token_k(self, value: int) -> str: ...
    def iter_session_events(self, session_path: Path) -> Iterator[JsonEvent]: ...


@dataclass(frozen=True, slots=True)
class WeeklyUsageScanResult:
    turns: int = 0
    token_events: int = 0
    total_tokens: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    files_scanned: int = 0
    recent_threads: frozenset[str] = field(default_factory=frozenset)
    latest_rate_limits: Mapping[str, JsonValue] | None = None
    latest_rate_limits_at: datetime | None = None


def scan_weekly_usage_events(
    sessions_dir: Path,
    cutoff: datetime,
    *,
    bridge_module: WeeklyUsageBridge,
) -> WeeklyUsageScanResult:
    turns = 0
    token_events = 0
    total_tokens = 0
    input_tokens = 0
    output_tokens = 0
    files_scanned = 0
    recent_threads: set[str] = set()
    latest_rate_limits: Mapping[str, JsonValue] | None = None
    latest_rate_limits_at: datetime | None = None

    for session_path in sessions_dir.rglob("*.jsonl"):
        files_scanned += 1
        try:
            for event in bridge_module.iter_session_events(session_path):
                moment = parse_event_timestamp(event.get("timestamp"))
                if moment is None or moment < cutoff:
                    continue
                payload = event.get("payload") or {}
                if not isinstance(payload, dict):
                    continue
                if event.get("type") == "session_meta":
                    thread_id = str(payload.get("id") or "").strip()
                    if thread_id:
                        recent_threads.add(thread_id)
                    continue
                if event.get("type") != "event_msg":
                    continue
                event_type = payload.get("type")
                if event_type == "task_started":
                    turns += 1
                    turn_id = str(payload.get("turn_id") or "").strip()
                    if turn_id:
                        recent_threads.add(turn_id)
                    continue
                if event_type != "token_count":
                    continue
                info = payload.get("info") or {}
                if not isinstance(info, dict):
                    continue
                rate_limits = payload.get("rate_limits")
                if isinstance(rate_limits, dict) and (
                    latest_rate_limits_at is None or moment > latest_rate_limits_at
                ):
                    latest_rate_limits = MappingProxyType(dict(rate_limits))
                    latest_rate_limits_at = moment
                last_usage = info.get("last_token_usage") or {}
                if not isinstance(last_usage, dict):
                    continue
                token_events += 1
                event_input = bridge_module.coerce_nonnegative_int(last_usage.get("input_tokens"))
                event_total = bridge_module.coerce_nonnegative_int(last_usage.get("total_tokens"))
                input_tokens += event_input
                total_tokens += event_total
                output_tokens += max(0, event_total - event_input)
        except OSError:
            continue

    return WeeklyUsageScanResult(
        turns=turns,
        token_events=token_events,
        total_tokens=total_tokens,
        input_tokens=input_tokens,
        output_tokens=output_tokens,
        files_scanned=files_scanned,
        recent_threads=frozenset(recent_threads),
        latest_rate_limits=latest_rate_limits,
        latest_rate_limits_at=latest_rate_limits_at,
    )

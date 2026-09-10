from __future__ import annotations

from pathlib import Path

import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_store as thread_store
from codex_session_events import iter_session_events
from codex_thread_context import thread_context_usage_from_events
from codex_thread_models import ThreadContextUsage, ThreadInfo
from codex_thread_settings import (
    collaboration_mode_from_events,
    format_thread_model_display as format_thread_settings_display,
    service_tier_from_events,
)


HIGH_CONTEXT_INPUT_RATIO_THRESHOLD = 0.70
CRITICAL_CONTEXT_INPUT_RATIO_THRESHOLD = 0.80
ARCHIVE_RECOMMEND_TOKENS_USED_THRESHOLD = 50_000_000
ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD = 200_000


def get_thread_context_usage(thread: ThreadInfo) -> ThreadContextUsage | None:
    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        return None
    return thread_context_usage_from_events(iter_session_events(session_path))


def get_thread_collaboration_mode(thread: ThreadInfo) -> str:
    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        return ""
    return collaboration_mode_from_events(iter_session_events(session_path))


def get_thread_service_tier(thread: ThreadInfo) -> str:
    session_path = Path(thread.rollout_path)
    speed = service_tier_from_events(iter_session_events(session_path)) if session_path.exists() else ""
    return speed or bridge_state.get_saved_thread_settings(thread.id).get("speed", "")


def format_thread_model_display(thread: ThreadInfo, mode: str, speed: str) -> str:
    return format_thread_settings_display(
        model=thread.model,
        reasoning=thread.reasoning_effort,
        mode=mode,
        speed=speed,
        saved_settings=bridge_state.get_saved_thread_settings(thread.id),
    )


def describe_thread_context_usage(context_usage: ThreadContextUsage) -> str:
    if context_usage.usage_ratio >= CRITICAL_CONTEXT_INPUT_RATIO_THRESHOLD:
        return "critical"
    if context_usage.usage_ratio >= HIGH_CONTEXT_INPUT_RATIO_THRESHOLD:
        return "high"
    return "normal"


def get_high_context_threads(limit: int = 20) -> list[tuple[ThreadInfo, ThreadContextUsage]]:
    flagged: list[tuple[ThreadInfo, ThreadContextUsage]] = []
    for thread in thread_store.load_recent_threads(limit=limit):
        context_usage = get_thread_context_usage(thread)
        if context_usage is None:
            continue
        if context_usage.usage_ratio >= HIGH_CONTEXT_INPUT_RATIO_THRESHOLD:
            flagged.append((thread, context_usage))

    flagged.sort(key=lambda item: (item[1].usage_ratio, item[0].updated_at), reverse=True)
    return flagged


def should_recommend_archive(thread: ThreadInfo, context_usage: ThreadContextUsage | None) -> bool:
    if thread.tokens_used >= ARCHIVE_RECOMMEND_TOKENS_USED_THRESHOLD:
        return True
    if context_usage is None:
        return False
    return (
        context_usage.last_input_tokens >= ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD
        or context_usage.peak_input_tokens >= ARCHIVE_RECOMMEND_CONTEXT_TOKENS_THRESHOLD
    )


def get_orphan_task_started_grace_seconds() -> float:
    return bridge_state.get_float_env(
        "CODEX_BRIDGE_ORPHAN_TASK_STARTED_GRACE_SECONDS",
        60.0,
        minimum=5.0,
        maximum=3600.0,
    )


def get_stale_busy_session_seconds() -> float:
    return bridge_state.get_float_env(
        "CODEX_BRIDGE_STALE_BUSY_SESSION_SECONDS",
        1800.0,
        minimum=60.0,
        maximum=86400.0,
    )

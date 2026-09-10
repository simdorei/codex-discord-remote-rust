from __future__ import annotations

from collections.abc import Iterable, Mapping
from dataclasses import dataclass

from codex_session_events import JsonEvent, JsonValue
from codex_thread_models import ThreadContextUsage


@dataclass(slots=True)  # noqa: MUTABLE_OK
class _ThreadContextUsageState:
    model_context_window: int = 0
    saw_token_count: bool = False
    last_input_tokens: int = 0
    last_total_tokens: int = 0
    peak_input_tokens: int = 0
    peak_total_tokens: int = 0
    previous_input_tokens: int = 0
    inferred_compactions: int = 0
    last_compaction_before_input_tokens: int = 0
    last_compaction_after_input_tokens: int = 0


def coerce_nonnegative_int(value: JsonValue | None) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return max(0, value)
    if isinstance(value, float):
        return max(0, int(value))
    if isinstance(value, str):
        try:
            return max(0, int(value))
        except ValueError:
            return 0
    return 0


def _mapping_value(value: JsonValue | None) -> Mapping[str, JsonValue] | None:
    return value if isinstance(value, dict) else None


def _apply_task_started_context(state: _ThreadContextUsageState, payload: Mapping[str, JsonValue]) -> None:
    state.model_context_window = (
        coerce_nonnegative_int(payload.get("model_context_window")) or state.model_context_window
    )


def _is_inferred_compaction(previous_input_tokens: int, last_input_tokens: int) -> bool:
    return (
        previous_input_tokens >= 50_000
        and last_input_tokens > 0
        and last_input_tokens < previous_input_tokens * 0.80
        and previous_input_tokens - last_input_tokens >= 25_000
    )


def _record_token_usage(
    state: _ThreadContextUsageState,
    last_usage: Mapping[str, JsonValue],
) -> None:
    state.saw_token_count = True
    state.last_input_tokens = coerce_nonnegative_int(last_usage.get("input_tokens"))
    state.last_total_tokens = coerce_nonnegative_int(last_usage.get("total_tokens"))
    if _is_inferred_compaction(state.previous_input_tokens, state.last_input_tokens):
        state.inferred_compactions += 1
        state.last_compaction_before_input_tokens = state.previous_input_tokens
        state.last_compaction_after_input_tokens = state.last_input_tokens
    if state.last_input_tokens > 0:
        state.previous_input_tokens = state.last_input_tokens
    state.peak_input_tokens = max(state.peak_input_tokens, state.last_input_tokens)
    state.peak_total_tokens = max(state.peak_total_tokens, state.last_total_tokens)


def _apply_token_count_context(state: _ThreadContextUsageState, payload: Mapping[str, JsonValue]) -> None:
    info = _mapping_value(payload.get("info"))
    if info is None:
        return
    state.model_context_window = (
        coerce_nonnegative_int(info.get("model_context_window")) or state.model_context_window
    )
    last_usage = _mapping_value(info.get("last_token_usage"))
    if last_usage is not None:
        _record_token_usage(state, last_usage)


def _usage_from_state(state: _ThreadContextUsageState) -> ThreadContextUsage | None:
    if not state.saw_token_count or state.model_context_window <= 0:
        return None
    usage_ratio = (
        min(1.0, state.last_input_tokens / state.model_context_window)
        if state.last_input_tokens
        else 0.0
    )
    return ThreadContextUsage(
        model_context_window=state.model_context_window,
        last_input_tokens=state.last_input_tokens,
        last_total_tokens=state.last_total_tokens,
        peak_input_tokens=state.peak_input_tokens,
        peak_total_tokens=state.peak_total_tokens,
        usage_ratio=usage_ratio,
        inferred_compactions=state.inferred_compactions,
        last_compaction_before_input_tokens=state.last_compaction_before_input_tokens,
        last_compaction_after_input_tokens=state.last_compaction_after_input_tokens,
    )


def thread_context_usage_from_events(events: Iterable[JsonEvent]) -> ThreadContextUsage | None:
    state = _ThreadContextUsageState()

    for event in events:
        payload = _mapping_value(event.get("payload"))
        if payload is None or event.get("type") != "event_msg":
            continue

        event_type = payload.get("type")
        if event_type == "task_started":
            _apply_task_started_context(state, payload)
            continue
        if event_type == "token_count":
            _apply_token_count_context(state, payload)

    return _usage_from_state(state)

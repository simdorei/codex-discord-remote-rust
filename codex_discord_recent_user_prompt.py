from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Protocol, TypeAlias

from codex_session_events import JsonEvent

NormalizePromptTextFunc = Callable[[str], str]
ExtractUserTextFunc = Callable[[JsonEvent], str]
ParseTimestampFunc = Callable[[JsonEvent], datetime | None]
ExceptionTypes: TypeAlias = tuple[type[BaseException], ...]
IterRecentSessionTailEventsFunc = Callable[[Path], list[JsonEvent]]
LogFunc = Callable[[str], None]
FormatExceptionFunc = Callable[[], str]


class RecentPromptThread(Protocol):
    rollout_path: str


ChooseThreadFunc = Callable[[str | None, str | None], RecentPromptThread]


@dataclass(frozen=True, slots=True)
class RecentCodexAppUserPromptDeps:
    choose_thread: ChooseThreadFunc
    iter_recent_session_tail_events: IterRecentSessionTailEventsFunc
    normalize_prompt_text: NormalizePromptTextFunc
    extract_user_text: ExtractUserTextFunc
    parse_timestamp: ParseTimestampFunc
    expected_exceptions: ExceptionTypes
    format_exception: FormatExceptionFunc
    log: LogFunc


def has_recent_user_prompt(
    events: list[JsonEvent],
    prompt: str,
    *,
    max_age_seconds: float,
    now: datetime,
    normalize_prompt_text_func: NormalizePromptTextFunc,
    extract_user_text_func: ExtractUserTextFunc,
    parse_timestamp_func: ParseTimestampFunc,
) -> bool:
    normalized_prompt = normalize_prompt_text_func(prompt)
    if not normalized_prompt:
        return False
    for event in reversed(events):
        user_text = extract_user_text_func(event)
        if not user_text:
            continue
        timestamp = parse_timestamp_func(event)
        if timestamp is None:
            continue
        age_seconds = max(0.0, (now - timestamp).total_seconds())
        if age_seconds > max_age_seconds:
            return False
        if normalize_prompt_text_func(user_text) == normalized_prompt:
            return True
    return False


def has_recent_codex_app_user_prompt(
    target_thread_id: str | None,
    prompt: str,
    *,
    max_age_seconds: float,
    deps: RecentCodexAppUserPromptDeps,
    now: datetime | None = None,
) -> bool:
    try:
        thread = deps.choose_thread(target_thread_id, None)
    except deps.expected_exceptions:
        deps.log(
            f"recent_codex_prompt_dedupe_unavailable target={target_thread_id or '-'} "
            + "reason=choose_thread_failed\n"
            + deps.format_exception()
        )
        return False
    return has_recent_user_prompt(
        deps.iter_recent_session_tail_events(Path(thread.rollout_path)),
        prompt,
        max_age_seconds=max_age_seconds,
        now=datetime.now(timezone.utc) if now is None else now,
        normalize_prompt_text_func=deps.normalize_prompt_text,
        extract_user_text_func=deps.extract_user_text,
        parse_timestamp_func=deps.parse_timestamp,
    )

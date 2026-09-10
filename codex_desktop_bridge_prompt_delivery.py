from __future__ import annotations

import re
import time
from collections.abc import Callable
from pathlib import Path
from typing import Final

from codex_bridge_state import JsonObject
from codex_pro_prompt_contract import CHROME_MENTION_NAME, CHROME_PLUGIN_URI
from codex_thread_models import ThreadInfo


ExtractMessageText = Callable[[JsonObject], str]
LoadRecentThreads = Callable[[int], list[ThreadInfo]]
ReadNewSessionEvents = Callable[[Path, int], tuple[list[JsonObject], int]]
SessionOffsets = dict[str, tuple[ThreadInfo, Path, int]]
Sleep = Callable[[float], None]
TimeNow = Callable[[], float]
_BROWSER_MENTION_PATTERN: Final[re.Pattern[str]] = re.compile(
    rf"\[@[^\]\r\n]+\]\({re.escape(CHROME_PLUGIN_URI)}\)",
    re.IGNORECASE,
)


def extract_user_text_from_event(
    event: JsonObject,
    *,
    extract_message_text: ExtractMessageText,
) -> str:
    payload = event.get("payload")
    if event.get("type") != "response_item" or not isinstance(payload, dict):
        return ""
    if payload.get("type") != "message" or payload.get("role") != "user":
        return ""
    return extract_message_text(payload)


def normalize_prompt_text(text: str) -> str:
    canonical_mentions = _BROWSER_MENTION_PATTERN.sub(
        f"[@{CHROME_MENTION_NAME}]({CHROME_PLUGIN_URI})",
        str(text),
    )
    return " ".join(canonical_mentions.replace("\r", " ").replace("\n", " ").split()).strip()


def snapshot_recent_session_offsets(
    *,
    limit: int,
    include_threads: list[ThreadInfo] | None,
    load_recent_threads: LoadRecentThreads,
) -> SessionOffsets:
    snapshot: SessionOffsets = {}
    threads = load_recent_threads(limit)
    if include_threads:
        seen_ids = {thread.id for thread in threads}
        for thread in include_threads:
            if thread.id not in seen_ids:
                threads.append(thread)
                seen_ids.add(thread.id)
    for thread in threads:
        session_path = Path(thread.rollout_path)
        if session_path.exists():
            snapshot[thread.id] = (thread, session_path, session_path.stat().st_size)
    return snapshot


def wait_for_prompt_delivery(
    session_offsets: SessionOffsets,
    prompt: str,
    timeout_sec: float = 4.0,
    *,
    read_new_session_events: ReadNewSessionEvents,
    extract_message_text: ExtractMessageText,
    time_now: TimeNow = time.time,
    sleep: Sleep = time.sleep,
) -> ThreadInfo | None:
    normalized_prompt = normalize_prompt_text(prompt)
    deadline = time_now() + timeout_sec
    cursors = {thread_id: offset for thread_id, (_, _, offset) in session_offsets.items()}

    while time_now() < deadline:
        for thread_id, (thread, session_path, _initial_offset) in session_offsets.items():
            cursor = cursors.get(thread_id, 0)
            events, cursor = read_new_session_events(session_path, cursor)
            cursors[thread_id] = cursor
            for event in events:
                user_text = extract_user_text_from_event(
                    event,
                    extract_message_text=extract_message_text,
                )
                if user_text and normalize_prompt_text(user_text) == normalized_prompt:
                    return thread
        sleep(0.2)

    return None

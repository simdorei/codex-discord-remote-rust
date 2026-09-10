from __future__ import annotations

from collections.abc import Callable
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Protocol, cast

from codex_session_events import JsonEvent, JsonValue
from codex_discord_context_refresh_items import (
    BuildInteractiveNoticeFunc as BuildInteractiveNoticeFunc,
    ExtractMessageTextFunc as ExtractMessageTextFunc,
    MakeSessionMirrorItemFunc as MakeSessionMirrorItemFunc,
    MakeTextDigestFunc as MakeTextDigestFunc,
    collect_context_refresh_items as collect_context_refresh_items,
    extract_context_refresh_item as extract_context_refresh_item,
    format_context_refresh_item as format_context_refresh_item,
    truncate_context_refresh_text as truncate_context_refresh_text,
)
from codex_discord_context_refresh_user_text import (
    extract_user_text_from_session_event as extract_user_text_from_session_event,
)
from codex_thread_models import ThreadInfo


class ContextRefreshBridge(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...
    def get_thread_workspace_ref(
        self,
        thread: ThreadInfo,
        threads: list[ThreadInfo] | None = None,
    ) -> str: ...
    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None: ...


GetMirroredThreadFunc = Callable[[int | None], str | None]
ResolveSelectedTargetFunc = Callable[[], tuple[str | None, str]]
IterRecentSessionTailEventsFunc = Callable[[Path], list[JsonEvent]]
CollectContextRefreshItemsFunc = Callable[[str, list[JsonEvent]], list[dict[str, str]]]
FormatContextRefreshItemFunc = Callable[[dict[str, str]], str]


def parse_session_event_timestamp(event: JsonEvent) -> datetime | None:
    raw_timestamp = event.get("timestamp")
    if not isinstance(raw_timestamp, str) or not raw_timestamp.strip():
        return None
    try:
        parsed = datetime.fromisoformat(raw_timestamp.strip().replace("Z", "+00:00"))
    except ValueError:
        return None
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def iter_recent_session_tail_events(session_path: Path, *, scan_bytes: int) -> list[JsonEvent]:
    if not session_path.exists():
        return []
    size = session_path.stat().st_size
    start = max(0, size - max(1, scan_bytes))
    with session_path.open("rb") as handle:
        _ = handle.seek(start)
        data = handle.read()
    lines = data.decode("utf-8", errors="replace").splitlines()
    if start > 0 and lines:
        lines = lines[1:]
    events: list[JsonEvent] = []
    for line in lines:
        try:
            event = cast(JsonValue, json.loads(line))
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def build_context_refresh_message(
    channel_id: int | None = None,
    *,
    limit: int,
    max_chars: int,
    bridge_module: ContextRefreshBridge,
    get_mirrored_codex_thread_id_func: GetMirroredThreadFunc,
    resolve_selected_target_func: ResolveSelectedTargetFunc,
    iter_recent_session_tail_events_func: IterRecentSessionTailEventsFunc,
    collect_context_refresh_items_func: CollectContextRefreshItemsFunc,
    format_context_refresh_item_func: FormatContextRefreshItemFunc,
) -> str:
    target_thread_id = get_mirrored_codex_thread_id_func(channel_id)
    if not target_thread_id:
        selected_thread_id, _target_ref = resolve_selected_target_func()
        target_thread_id = selected_thread_id
    if not target_thread_id:
        return "No Codex thread target found."
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
    except (RuntimeError, OSError) as exc:
        return f"Context refresh unavailable.\n\nERROR: {exc}"

    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        return f"Context refresh unavailable.\n\nSession file not found: {session_path}"

    events = iter_recent_session_tail_events_func(session_path)
    items = collect_context_refresh_items_func(target_thread_id, events)[-limit:]
    title = bridge_module.get_thread_ui_name(thread.id, thread) or thread.title or "-"
    header = [
        "Context refresh",
        f"thread_ref: {bridge_module.get_thread_workspace_ref(thread, [thread])}",
        f"title: {title}",
        f"items: {len(items)}/{limit}",
        "source: recent session tail",
    ]
    if not items:
        output = "\n".join(header + ["", "(no recent text items found)"])
    else:
        output = "\n\n".join(
            ["\n".join(header), *[format_context_refresh_item_func(item) for item in items]]
        )

    max_chars = max(1000, min(50000, int(max_chars)))
    if len(output) <= max_chars:
        return output
    marker = "\n\n[context refresh truncated]"
    return output[: max_chars - len(marker)].rstrip() + marker

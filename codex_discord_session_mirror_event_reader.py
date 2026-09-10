from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, TypeVar

EventT = TypeVar("EventT")
LogFunc = Callable[[str], None]
ArchiveBacklogMaxGetter = Callable[[], int]
ReadEventsFunc = Callable[[Path, int, int | None], Awaitable[tuple[list[EventT], int]]]


@dataclass(frozen=True, slots=True)
class SessionMirrorEventReadResult(Generic[EventT]):
    events: list[EventT]
    next_cursor: int


@dataclass(frozen=True, slots=True)
class SessionMirrorEventReaderDeps(Generic[EventT]):
    read_events: ReadEventsFunc[EventT]
    get_archive_backlog_max_events: ArchiveBacklogMaxGetter
    log: LogFunc


async def read_session_mirror_events(
    codex_thread_id: str,
    session_path: Path,
    cursor: int,
    *,
    archive_tail_only: bool,
    deps: SessionMirrorEventReaderDeps[EventT],
) -> SessionMirrorEventReadResult[EventT]:
    max_events = deps.get_archive_backlog_max_events() if archive_tail_only else 0
    events, next_cursor = await deps.read_events(
        session_path,
        cursor,
        max_events if max_events else None,
    )
    if archive_tail_only and events:
        max_events_text = str(max_events) if max_events else "unlimited"
        message = f"session_mirror_archive_backlog_batch target={codex_thread_id} events={len(events)} max_events={max_events_text}"
        deps.log(message)
    return SessionMirrorEventReadResult(events=events, next_cursor=next_cursor)

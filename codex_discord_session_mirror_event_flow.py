from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, TypeVar

import codex_discord_session_mirror_archive as session_mirror_archive
import codex_discord_session_mirror_cursor as session_mirror_cursor
import codex_discord_session_mirror_event_reader as session_mirror_event_reader
import codex_discord_session_mirror_event_policy as session_mirror_event_policy
import codex_discord_session_mirror_items as session_mirror_items

ThreadT = TypeVar("ThreadT")
ContextUsageT = TypeVar("ContextUsageT")
EventT = TypeVar("EventT")
ItemT = TypeVar("ItemT")


@dataclass(frozen=True, slots=True)
class SessionMirrorPreparedItems(Generic[EventT, ItemT]):
    rollout_path: str
    events: list[EventT]
    items: list[ItemT]
    next_cursor: int


SessionMirrorThreadPolicy = session_mirror_event_policy.SessionMirrorThreadPolicy


@dataclass(frozen=True, slots=True)
class SessionMirrorEventFlowDeps(Generic[ThreadT, ContextUsageT, EventT, ItemT]):
    choose_thread: Callable[[str], Awaitable[ThreadT]]
    get_thread_context_usage: Callable[[ThreadT], Awaitable[ContextUsageT]]
    should_recommend_archive: Callable[[ThreadT, ContextUsageT], bool]
    get_thread_rollout_path: Callable[[ThreadT], str]
    is_active_output_target: Callable[[str], bool]
    archive_skip_logged: set[str]
    is_pending_cursor_target: session_mirror_cursor.PendingCursorPredicate
    clear_pending_cursor_target: session_mirror_cursor.PendingCursorClearer
    update_session_mirror_cursor: session_mirror_cursor.SessionMirrorCursorUpdater
    get_or_init_session_mirror_cursor: session_mirror_cursor.SessionMirrorCursorGetter
    read_events: session_mirror_event_reader.ReadEventsFunc[EventT]
    get_archive_backlog_max_events: session_mirror_event_reader.ArchiveBacklogMaxGetter
    collect_session_mirror_items: session_mirror_items.SessionMirrorItemCollector[EventT, ItemT]
    get_seen_agent_messages: Callable[[str], session_mirror_items.SeenMessages]
    get_seen_user_messages: Callable[[str], session_mirror_items.SeenMessages]
    log: session_mirror_archive.LogFunc


async def _prepare_session_mirror_thread_policy(
    codex_thread_id: str,
    *,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> SessionMirrorThreadPolicy[ThreadT] | None:
    return await session_mirror_event_policy.prepare_session_mirror_thread_policy(
        codex_thread_id,
        deps=session_mirror_event_policy.SessionMirrorEventPolicyDeps(
            choose_thread=deps.choose_thread,
            get_thread_context_usage=deps.get_thread_context_usage,
            should_recommend_archive=deps.should_recommend_archive,
            is_active_output_target=deps.is_active_output_target,
            archive_skip_logged=deps.archive_skip_logged,
            log=deps.log,
        ),
    )


def _existing_session_path(
    codex_thread: ThreadT,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> Path | None:
    session_path = Path(deps.get_thread_rollout_path(codex_thread))
    if not session_path.exists():
        return None
    return session_path


async def _initialize_session_mirror_event_cursor(
    codex_thread_id: str,
    session_path: Path,
    *,
    active_output_target: bool,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> int:
    return await session_mirror_cursor.initialize_session_mirror_cursor(
        codex_thread_id,
        str(session_path),
        session_size=session_path.stat().st_size,
        active_output_target=active_output_target,
        deps=session_mirror_cursor.SessionMirrorCursorInitDeps(
            is_pending_cursor_target=deps.is_pending_cursor_target,
            clear_pending_cursor_target=deps.clear_pending_cursor_target,
            update_session_mirror_cursor=deps.update_session_mirror_cursor,
            get_or_init_session_mirror_cursor=deps.get_or_init_session_mirror_cursor,
            log=deps.log,
        ),
    )


async def _read_session_mirror_event_batch(
    codex_thread_id: str,
    session_path: Path,
    cursor: int,
    *,
    archive_tail_only: bool,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> session_mirror_event_reader.SessionMirrorEventReadResult[EventT]:
    return await session_mirror_event_reader.read_session_mirror_events(
        codex_thread_id,
        session_path,
        cursor,
        archive_tail_only=archive_tail_only,
        deps=session_mirror_event_reader.SessionMirrorEventReaderDeps(
            read_events=deps.read_events,
            get_archive_backlog_max_events=deps.get_archive_backlog_max_events,
            log=deps.log,
        ),
    )


async def _collect_session_mirror_delivery_batch(
    codex_thread_id: str,
    events: list[EventT],
    rollout_path: str,
    next_cursor: int,
    *,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> session_mirror_items.SessionMirrorItemsResult[ItemT]:
    return await session_mirror_items.collect_session_mirror_delivery_items(
        codex_thread_id,
        events,
        rollout_path,
        next_cursor,
        seen_agent_messages=deps.get_seen_agent_messages(codex_thread_id),
        seen_user_messages=deps.get_seen_user_messages(codex_thread_id),
        deps=session_mirror_items.SessionMirrorItemsDeps(
            collect_session_mirror_items=deps.collect_session_mirror_items,
            update_session_mirror_cursor=deps.update_session_mirror_cursor,
        ),
    )


async def prepare_session_mirror_delivery_items(
    codex_thread_id: str,
    *,
    deps: SessionMirrorEventFlowDeps[ThreadT, ContextUsageT, EventT, ItemT],
) -> SessionMirrorPreparedItems[EventT, ItemT] | None:
    policy = await _prepare_session_mirror_thread_policy(codex_thread_id, deps=deps)
    if policy is None:
        return None

    session_path = _existing_session_path(policy.codex_thread, deps)
    if session_path is None:
        return None

    rollout_path = str(session_path)
    cursor = await _initialize_session_mirror_event_cursor(
        codex_thread_id,
        session_path,
        active_output_target=policy.active_output_target,
        deps=deps,
    )
    event_read = await _read_session_mirror_event_batch(
        codex_thread_id,
        session_path,
        cursor,
        archive_tail_only=policy.archive_tail_only,
        deps=deps,
    )
    if not event_read.events:
        return None

    item_collection = await _collect_session_mirror_delivery_batch(
        codex_thread_id,
        event_read.events,
        rollout_path,
        event_read.next_cursor,
        deps=deps,
    )
    if not item_collection.items:
        return None

    return SessionMirrorPreparedItems(
        rollout_path=rollout_path,
        events=event_read.events,
        items=item_collection.items,
        next_cursor=event_read.next_cursor,
    )

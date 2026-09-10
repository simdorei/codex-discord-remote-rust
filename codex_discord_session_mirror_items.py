from __future__ import annotations

import asyncio  # noqa: ANYIO_OK

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

EventT = TypeVar("EventT")
ItemT = TypeVar("ItemT")
SeenMessages = dict[str, float]
CursorUpdater = Callable[[str, str, int], Awaitable[None]]


class SessionMirrorItemCollector(Protocol[EventT, ItemT]):
    def __call__(
        self,
        codex_thread_id: str,
        events: list[EventT],
        *,
        seen_agent_messages: SeenMessages,
        seen_user_messages: SeenMessages,
    ) -> list[ItemT]: ...


@dataclass(frozen=True, slots=True)
class SessionMirrorItemsResult(Generic[ItemT]):
    items: list[ItemT]
    cursor_committed: bool


@dataclass(frozen=True, slots=True)
class SessionMirrorItemsDeps(Generic[EventT, ItemT]):
    collect_session_mirror_items: SessionMirrorItemCollector[EventT, ItemT]
    update_session_mirror_cursor: CursorUpdater


async def collect_session_mirror_delivery_items(
    codex_thread_id: str,
    events: list[EventT],
    rollout_path: str,
    next_cursor: int,
    *,
    seen_agent_messages: SeenMessages,
    seen_user_messages: SeenMessages,
    deps: SessionMirrorItemsDeps[EventT, ItemT],
) -> SessionMirrorItemsResult[ItemT]:
    items = await asyncio.to_thread(
        deps.collect_session_mirror_items,
        codex_thread_id,
        events,
        seen_agent_messages=seen_agent_messages,
        seen_user_messages=seen_user_messages,
    )
    if not items:
        await deps.update_session_mirror_cursor(codex_thread_id, rollout_path, next_cursor)
        return SessionMirrorItemsResult(items=[], cursor_committed=True)
    return SessionMirrorItemsResult(items=items, cursor_committed=False)

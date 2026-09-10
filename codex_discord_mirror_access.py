from __future__ import annotations

from collections.abc import AsyncIterator, Iterable
from dataclasses import dataclass
from importlib import import_module
from types import ModuleType
from typing import Final, Protocol, TypeAlias, TypeGuard


ACCESS_UNKNOWN: Final = "unknown"
ACCESS_TRUE: Final = "true"
ACCESS_FALSE: Final = "false"
ARCHIVED_UNKNOWN: Final = "unknown"
ARCHIVED_TRUE: Final = "true"
ARCHIVED_FALSE: Final = "false"
ACTIVE_MAPPING_REASON: Final = "active_mapping"
FETCH_FAILED_REASON: Final = "fetch_failed"
FORBIDDEN_REASON: Final = "forbidden"
NOT_FOUND_REASON: Final = "not_found"
NOT_IN_THREAD_LISTS_REASON: Final = "not_in_active_or_archived_thread_lists"
UNKNOWN_CHANNEL_REASON: Final = "unknown_channel"


class ThreadAccessTarget(Protocol):
    discord_thread_id: int
    parent_channel_id: int


class MirrorAccessBot(Protocol):
    def get_channel(self, channel_id: int) -> MirrorChannel | None:
        ...

    async def fetch_channel(self, channel_id: int) -> MirrorChannel:
        ...


class ChannelLike(Protocol):
    @property
    def id(self) -> int | str:
        ...


class ThreadLike(ChannelLike, Protocol):
    @property
    def archived(self) -> bool:
        ...


class ParentChannelLike(ChannelLike, Protocol):
    @property
    def threads(self) -> Iterable[ThreadLike]:
        ...


class ArchivedThreadProvider(ParentChannelLike, Protocol):
    def archived_threads(self, *, limit: int) -> AsyncIterator[ThreadLike]:
        ...


MirrorChannel: TypeAlias = ThreadLike | ParentChannelLike


@dataclass(frozen=True, slots=True)
class MirrorThreadAccessStatus:
    accessible: str
    archived: str
    stale: bool
    reason: str


def bool_label(value: bool) -> str:
    return "true" if value else "false"


def _exception_type(module: ModuleType, name: str) -> type[Exception] | None:
    value = getattr(module, name, None)
    if not isinstance(value, type):
        return None
    if not issubclass(value, Exception):
        return None
    return value


def _discord_exception_types() -> tuple[type[Exception], ...]:
    try:
        discord_module = import_module("discord")
    except ModuleNotFoundError:
        return ()

    found: list[type[Exception]] = []
    for name in ("DiscordException", "HTTPException", "Forbidden", "NotFound"):
        exc_type = _exception_type(discord_module, name)
        if exc_type is not None and exc_type not in found:
            found.append(exc_type)
    return tuple(found)


def accessibility_reason_from_fetch_error(exc: Exception) -> str:
    error_type = type(exc).__name__
    message = str(exc)
    if "Unknown Channel" in message:
        return UNKNOWN_CHANNEL_REASON
    if error_type == "Forbidden":
        return FORBIDDEN_REASON
    if error_type == "NotFound":
        return NOT_FOUND_REASON
    return FETCH_FAILED_REASON


def not_in_active_or_archived_thread_lists_reason() -> str:
    return NOT_IN_THREAD_LISTS_REASON


def _thread_id(value: ChannelLike) -> int | None:
    raw_id = value.id
    if isinstance(raw_id, int):
        return raw_id
    try:
        return int(raw_id)
    except ValueError:
        return None


def _status_for_thread(thread: ThreadLike) -> MirrorThreadAccessStatus:
    archived = bool(thread.archived)
    return MirrorThreadAccessStatus(
        accessible=ACCESS_TRUE,
        archived=ARCHIVED_TRUE if archived else ARCHIVED_FALSE,
        stale=False,
        reason=ACTIVE_MAPPING_REASON,
    )


async def _fetch_channel(bot: MirrorAccessBot, channel_id: int) -> MirrorChannel | Exception:
    try:
        return await bot.fetch_channel(channel_id)
    except _discord_exception_types() as exc:
        return exc


def _is_thread_channel(value: MirrorChannel) -> TypeGuard[ThreadLike]:
    return hasattr(value, "archived")


def _is_parent_channel(value: MirrorChannel) -> TypeGuard[ParentChannelLike]:
    return hasattr(value, "threads")


def _has_archived_threads(value: ParentChannelLike) -> TypeGuard[ArchivedThreadProvider]:
    return callable(getattr(value, "archived_threads", None))


def _find_active_thread(parent_channel: ParentChannelLike, thread_id: int) -> ThreadLike | None:
    for thread in parent_channel.threads:
        if _thread_id(thread) == thread_id:
            return thread
    return None


async def _find_archived_thread(parent_channel: ParentChannelLike, thread_id: int) -> ThreadLike | None:
    if not _has_archived_threads(parent_channel):
        return None
    async for thread in parent_channel.archived_threads(limit=100):
        if _thread_id(thread) == thread_id:
            return thread
    return None


async def _resolve_parent_channel(
    bot: MirrorAccessBot,
    parent_channel_id: int,
) -> MirrorChannel | Exception | None:
    cached = bot.get_channel(parent_channel_id)
    if cached is not None:
        return cached
    return await _fetch_channel(bot, parent_channel_id)


def _stale_status(reason: str) -> MirrorThreadAccessStatus:
    return MirrorThreadAccessStatus(
        accessible=ACCESS_FALSE,
        archived=ARCHIVED_UNKNOWN,
        stale=True,
        reason=reason,
    )


async def inspect_thread_access(
    bot: MirrorAccessBot,
    *,
    parent_channel_id: int,
    discord_thread_id: int,
) -> MirrorThreadAccessStatus:
    fetched = await _fetch_channel(bot, discord_thread_id)
    fetch_reason = ""
    if isinstance(fetched, Exception):
        fetch_reason = accessibility_reason_from_fetch_error(fetched)
    elif _is_thread_channel(fetched) and _thread_id(fetched) == discord_thread_id:
        return _status_for_thread(fetched)

    parent = await _resolve_parent_channel(bot, parent_channel_id)
    if isinstance(parent, Exception):
        return _stale_status(fetch_reason or accessibility_reason_from_fetch_error(parent))
    if parent is None:
        return _stale_status(fetch_reason or NOT_IN_THREAD_LISTS_REASON)
    if not _is_parent_channel(parent):
        return _stale_status(fetch_reason or NOT_IN_THREAD_LISTS_REASON)

    active_thread = _find_active_thread(parent, discord_thread_id)
    if active_thread is not None:
        return _status_for_thread(active_thread)

    archived_thread = await _find_archived_thread(parent, discord_thread_id)
    if archived_thread is not None:
        return MirrorThreadAccessStatus(
            accessible=ACCESS_TRUE,
            archived=ARCHIVED_TRUE,
            stale=False,
            reason=ACTIVE_MAPPING_REASON,
        )

    return _stale_status(fetch_reason or NOT_IN_THREAD_LISTS_REASON)


async def inspect_thread_access_map(
    bot: MirrorAccessBot,
    targets: Iterable[ThreadAccessTarget],
) -> dict[int, MirrorThreadAccessStatus]:
    statuses: dict[int, MirrorThreadAccessStatus] = {}
    for target in targets:
        thread_id = int(target.discord_thread_id)
        statuses[thread_id] = await inspect_thread_access(
            bot,
            parent_channel_id=int(target.parent_channel_id),
            discord_thread_id=thread_id,
        )
    return statuses

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

ChannelT = TypeVar("ChannelT")
CachedChannelGetter = Callable[[int], tuple[ChannelT | None, str]]
ChannelFetcher = Callable[[int], Awaitable[ChannelT]]
FetchFailureTypes = tuple[type[Exception], ...]
MessageablePredicate = Callable[[ChannelT], bool]
LogFunc = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class SessionMirrorChannelResolveDeps(Generic[ChannelT]):
    get_cached_channel_or_thread: CachedChannelGetter[ChannelT]
    fetch_channel: ChannelFetcher[ChannelT]
    fetch_failure_types: FetchFailureTypes
    is_messageable: MessageablePredicate[ChannelT]
    log: LogFunc


async def resolve_session_mirror_channel(
    discord_thread_id: int,
    *,
    deps: SessionMirrorChannelResolveDeps[ChannelT],
) -> ChannelT | None:
    channel_id = int(discord_thread_id)
    channel, source = deps.get_cached_channel_or_thread(channel_id)
    if channel is None:
        try:
            channel = await deps.fetch_channel(channel_id)
            source = "fetch"
        except deps.fetch_failure_types as exc:
            message = f"session_mirror_channel_failed channel={channel_id} error_type={type(exc).__name__}"
            deps.log(message)
            return None
    if not deps.is_messageable(channel):
        message = f"session_mirror_channel_skipped channel={channel_id} source={source} reason=not_messageable"
        deps.log(message)
        return None
    return channel

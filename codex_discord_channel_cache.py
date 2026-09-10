from __future__ import annotations

from collections.abc import Iterable
from typing import Protocol, TypeVar

ChannelT = TypeVar("ChannelT", covariant=True)


class GuildChannelCache(Protocol[ChannelT]):
    def get_thread(self, channel_id: int, /) -> ChannelT | None: ...

    def get_channel(self, channel_id: int, /) -> ChannelT | None: ...


class ClientChannelCache(Protocol[ChannelT]):
    @property
    def guilds(self) -> Iterable[GuildChannelCache[ChannelT]]: ...

    def get_channel(self, channel_id: int, /) -> ChannelT | None: ...


def get_cached_channel_or_thread(
    client: ClientChannelCache[ChannelT],
    channel_id: int,
) -> tuple[ChannelT | None, str]:
    channel = client.get_channel(channel_id)
    if channel is not None:
        return channel, "client_channel_cache"
    for guild in client.guilds:
        thread = guild.get_thread(channel_id)
        if thread is not None:
            return thread, "guild_thread_cache"
        guild_channel = guild.get_channel(channel_id)
        if guild_channel is not None:
            return guild_channel, "guild_channel_cache"
    return None, "-"

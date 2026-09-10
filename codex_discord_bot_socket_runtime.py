from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

import codex_discord_seen_cache as discord_seen_cache
import codex_discord_socket_event_log as discord_socket_event_log
ModuleValue: TypeAlias = object


SocketEventPayload = dict[str, discord_socket_event_log.SocketEventValue]
SocketEventData = discord_socket_event_log.SocketEventData


class SocketRuntimeOwner(discord_seen_cache.SeenCacheOwner, Protocol):
    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[ModuleValue | None, str]: ...

    def is_allowed_message_channel(self, channel: ModuleValue) -> bool: ...

    def is_allowed_channel(self, channel_id: int | None) -> bool: ...


@dataclass(frozen=True, slots=True)
class BotSocketRuntimeDeps:
    socket_event_log_id_limit: int
    is_mirrored_channel_id: Callable[[int | None], bool]
    delivery_exceptions: tuple[type[BaseException], ...]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class BotSocketRuntime:
    deps: BotSocketRuntimeDeps

    async def on_socket_raw_receive(self, owner: SocketRuntimeOwner, message: str | bytes) -> None:
        payload = discord_socket_event_log.parse_raw_socket_payload(message)
        if payload is None:
            return
        await self.log_socket_payload(owner, payload)

    async def on_socket_response(self, owner: SocketRuntimeOwner, payload: SocketEventPayload) -> None:
        await self.log_socket_payload(owner, payload)

    def is_tracked_socket_message_channel(
        self,
        owner: SocketRuntimeOwner,
        channel_id: int | None,
    ) -> tuple[bool, str]:
        return discord_socket_event_log.track_socket_message_channel(
            channel_id,
            get_cached_channel_or_thread=owner.get_cached_channel_or_thread,
            is_allowed_message_channel=owner.is_allowed_message_channel,
            is_allowed_channel=owner.is_allowed_channel,
            is_mirrored_channel_id=self.deps.is_mirrored_channel_id,
            cache_error_exceptions=self.deps.delivery_exceptions,
        )

    async def log_socket_payload(self, owner: SocketRuntimeOwner, payload: SocketEventData) -> None:
        for line in discord_socket_event_log.format_socket_payload_log_lines(
            payload,
            claim_event=lambda event_payload: discord_socket_event_log.claim_socket_event_log(
                owner,
                event_payload,
                limit=self.deps.socket_event_log_id_limit,
            ),
            track_message_channel=lambda channel_id: self.is_tracked_socket_message_channel(owner, channel_id),
        ):
            self.deps.log(line)

    def format_socket_interaction_user(self, data: SocketEventData) -> str:
        return discord_socket_event_log.format_socket_interaction_user(data)

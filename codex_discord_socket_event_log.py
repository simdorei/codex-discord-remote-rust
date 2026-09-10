from __future__ import annotations

import json
from collections.abc import Callable, Mapping
from typing import TypeAlias, cast

import codex_discord_interaction_log as discord_interaction_log
import codex_discord_seen_cache as seen_cache
from codex_discord_text import format_log_text_len

SocketEventScalar: TypeAlias = str | int | bool | None
SocketEventData: TypeAlias = Mapping[str, "SocketEventValue"]
SocketEventValue: TypeAlias = SocketEventScalar | SocketEventData
SeenCacheMap: TypeAlias = seen_cache.SeenCacheMap
ClaimSocketEventLog: TypeAlias = Callable[[SocketEventData], bool]
TrackSocketMessageChannel: TypeAlias = Callable[[int | None], tuple[bool, str]]
GetCachedSocketChannel: TypeAlias = Callable[[int], tuple[object | None, str]]
IsAllowedSocketMessageChannel: TypeAlias = Callable[[object], bool]
IsAllowedSocketChannelId: TypeAlias = Callable[[int | None], bool]


def parse_raw_socket_payload(message: str | bytes) -> SocketEventData | None:
    if isinstance(message, bytes):
        raw_text = message.decode("utf-8", errors="replace")
    else:
        raw_text = str(message)
    try:
        payload = cast(object, json.loads(raw_text))
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, Mapping):
        return None
    return cast(SocketEventData, payload)


def track_socket_message_channel(
    channel_id: int | None,
    *,
    get_cached_channel_or_thread: GetCachedSocketChannel,
    is_allowed_message_channel: IsAllowedSocketMessageChannel,
    is_allowed_channel: IsAllowedSocketChannelId,
    is_mirrored_channel_id: IsAllowedSocketChannelId,
    cache_error_exceptions: tuple[type[BaseException], ...],
) -> tuple[bool, str]:
    if channel_id is None:
        return False, "missing_channel"
    channel, source = get_cached_channel_or_thread(channel_id)
    if channel is not None:
        try:
            if is_allowed_message_channel(channel):
                return True, source
        except cache_error_exceptions:
            return False, "cache_error"
    if is_allowed_channel(channel_id):
        return True, "allowed_channel_id"
    if is_mirrored_channel_id(channel_id):
        return True, "mirror_channel_id"
    return False, source


def get_socket_event_log_key(payload: Mapping[str, SocketEventValue]) -> str | None:
    event_type = str(payload.get("t") or "").strip()
    if not event_type:
        return None
    sequence = payload.get("s")
    if sequence is not None:
        return f"{event_type}:s:{sequence}"
    data = payload.get("d")
    if isinstance(data, Mapping):
        event_id = data.get("id")
        if event_id is not None:
            return f"{event_type}:id:{event_id}"
    return None


def format_socket_interaction_user(data: Mapping[str, SocketEventValue]) -> str:
    user = data.get("user")
    if isinstance(user, Mapping) and user.get("id"):
        return str(user.get("id"))
    member = data.get("member")
    if isinstance(member, Mapping):
        member_user = member.get("user")
        if isinstance(member_user, Mapping) and member_user.get("id"):
            return str(member_user.get("id"))
    return "-"


def claim_socket_event_log(
    owner: seen_cache.SeenCacheOwner,
    payload: SocketEventData,
    *,
    limit: int,
) -> bool:
    event_key = get_socket_event_log_key(payload)
    if event_key is None:
        return True
    seen = seen_cache.get_or_create_seen_map(owner, "_logged_socket_event_ids")
    if seen is None:
        return True
    if event_key in seen:
        return False
    seen_cache.remember_limited_seen_key(seen, event_key, limit=limit)
    return True


def format_socket_payload_log_lines(
    payload: SocketEventData,
    *,
    claim_event: ClaimSocketEventLog,
    track_message_channel: TrackSocketMessageChannel,
) -> tuple[str, ...]:
    event_type = str(payload.get("t") or "")
    data = payload.get("d")
    if not isinstance(data, Mapping):
        return ()
    event_data = data
    if not claim_event(payload):
        return ()
    if event_type == "MESSAGE_CREATE":
        return (_format_message_create_log_line(event_data, track_message_channel),)
    if event_type == "INTERACTION_CREATE":
        return (_format_interaction_create_log_line(event_data),)
    return ()


def _format_message_create_log_line(
    data: SocketEventData,
    track_message_channel: TrackSocketMessageChannel,
) -> str:
    channel_id = _socket_channel_id(data.get("channel_id"))
    author = data.get("author")
    author_id = "-"
    author_bot = "-"
    if isinstance(author, Mapping):
        author_id = str(author.get("id") or "-")
        author_bot = str(bool(author.get("bot", False)))
    tracked, track_source = track_message_channel(channel_id)
    if not tracked:
        return (
            f"socket_message_create_untracked channel={channel_id or '-'} "
            + f"guild={data.get('guild_id') or '-'} source={track_source}"
        )
    return (
        f"socket_message_create channel={channel_id or '-'} tracked={tracked} "
        + f"source={track_source} guild={data.get('guild_id') or '-'} "
        + f"author={author_id} bot={author_bot} content_len={format_log_text_len(_socket_content(data))}"
    )


def _format_interaction_create_log_line(data: SocketEventData) -> str:
    channel_id = data.get("channel_id") or "-"
    return (
        f"socket_interaction_create channel={channel_id} guild={data.get('guild_id') or '-'} "
        + f"user={format_socket_interaction_user(data)} "
        + f"type={data.get('type') or '-'} "
        + "command="
        + discord_interaction_log.format_raw_interaction_command(
            cast(discord_interaction_log.RawInteractionData, data)
        )
    )


def _socket_channel_id(value: SocketEventValue | None) -> int | None:
    try:
        return int(str(value))
    except ValueError:
        return None


def _socket_content(data: SocketEventData) -> str | None:
    content = data.get("content")
    return content if isinstance(content, str) else None

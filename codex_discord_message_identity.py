from __future__ import annotations

from typing import Protocol, TypeAlias


class SupportsInt(Protocol):
    def __int__(self) -> int: ...


RawDiscordMessageId: TypeAlias = str | bytes | bytearray | SupportsInt | None


def coerce_discord_message_id(raw_id: RawDiscordMessageId) -> int | None:
    if raw_id is None:
        return None
    try:
        return int(raw_id)
    except (TypeError, ValueError):
        return None

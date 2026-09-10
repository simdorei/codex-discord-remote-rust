from __future__ import annotations

from typing import TypeAlias

DiscordIdValue: TypeAlias = int | str | bytes | bytearray | None


def coerce_discord_id_value(discord_id: DiscordIdValue) -> int | None:
    if discord_id is None:
        return None
    if isinstance(discord_id, int):
        return int(discord_id)
    if isinstance(discord_id, str):
        raw = discord_id
    else:
        try:
            raw = bytes(discord_id).decode("ascii")
        except UnicodeDecodeError:
            return None
    try:
        return int(raw)
    except ValueError:
        return None

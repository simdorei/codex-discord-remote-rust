from __future__ import annotations

from codex_discord_commands import parse_bounded_int


def clamp_context_refresh_limit(
    raw: str | None,
    *,
    default: int,
    minimum: int,
    maximum: int,
) -> int:
    return parse_bounded_int(raw, default=default, minimum=minimum, maximum=maximum)

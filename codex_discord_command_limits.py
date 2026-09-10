from __future__ import annotations

from codex_discord_command_types import MirrorAction, PrefixLimitAction, RawCommandValue


def parse_bounded_int(raw: RawCommandValue, *, default: int, minimum: int, maximum: int) -> int:
    if raw is None or str(raw).strip() == "":
        return default
    try:
        value = int(str(raw).strip())
    except ValueError:
        return default
    return max(minimum, min(maximum, value))


def parse_required_bounded_int(raw: RawCommandValue, *, default: int, minimum: int, maximum: int) -> int | None:
    if raw is None or str(raw).strip() == "":
        return default
    try:
        value = int(str(raw).strip())
    except ValueError:
        return None
    return max(minimum, min(maximum, value))


def parse_usage_days(arg: str) -> PrefixLimitAction:
    days = parse_required_bounded_int(arg, default=7, minimum=1, maximum=30)
    if days is None:
        return PrefixLimitAction(None, "Usage: !usage [days]")
    return PrefixLimitAction(days)


def parse_bridge_sync_limit(command: str, arg: str) -> PrefixLimitAction:
    if command == "bridge":
        subcommand, _, subarg = arg.partition(" ")
        if (subcommand or "sync").lower().strip() != "sync":
            return PrefixLimitAction(None, "Usage: !bridge sync [limit]")
        limit_arg = subarg.strip()
    else:
        limit_arg = arg.strip()
    if not limit_arg:
        return PrefixLimitAction(None)
    limit = parse_required_bounded_int(limit_arg, default=1, minimum=1, maximum=100)
    if limit is None:
        return PrefixLimitAction(None, "Usage: !bridge sync [limit]")
    return PrefixLimitAction(limit)


def parse_mirror_action(arg: str) -> MirrorAction:
    subcommand, _, subarg = arg.partition(" ")
    subcommand = (subcommand or "sync").lower().strip()
    subarg = subarg.strip()
    if subcommand == "sync":
        if subarg:
            return MirrorAction(None, usage="Usage: !mirror sync")
        return MirrorAction("sync")
    if subcommand == "list":
        if not subarg:
            return MirrorAction("list")
        limit = parse_required_bounded_int(subarg, default=1, minimum=1, maximum=100)
        if limit is None:
            return MirrorAction(None, usage="Usage: !mirror list [limit]")
        return MirrorAction("list", limit=limit)
    if subcommand in {"check", "doctor"}:
        if not subarg:
            return MirrorAction("check")
        limit = parse_required_bounded_int(subarg, default=1, minimum=1, maximum=100)
        if limit is None:
            return MirrorAction(None, usage="Usage: !mirror check [limit]")
        return MirrorAction("check", limit=limit)
    return MirrorAction(None, usage="Usage: !mirror sync | !mirror list [limit] | !mirror check [limit]")

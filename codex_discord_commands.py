"""Pure command parsing and argv builders for the Discord adapter."""

from __future__ import annotations

from codex_discord_command_bridge_actions import (
    build_archive_argv as build_archive_argv,
    build_archived_list_argv as build_archived_list_argv,
    build_list_argv as build_list_argv,
    build_open_argv as build_open_argv,
    build_prefix_bridge_action as build_prefix_bridge_action,
    build_settings_argv as build_settings_argv,
    build_settings_values_argv as build_settings_values_argv,
    build_status_argv as build_status_argv,
)
from codex_discord_command_limits import (
    parse_bounded_int as parse_bounded_int,
    parse_bridge_sync_limit as parse_bridge_sync_limit,
    parse_mirror_action as parse_mirror_action,
    parse_required_bounded_int as parse_required_bounded_int,
    parse_usage_days as parse_usage_days,
)
from codex_discord_command_types import (
    MirrorAction as MirrorAction,
    PrefixBridgeAction as PrefixBridgeAction,
    PrefixCommand as PrefixCommand,
    PrefixLimitAction as PrefixLimitAction,
    RawCommandValue as RawCommandValue,
)
import codex_discord_settings_commands as settings_commands

SETTINGS_USAGE = settings_commands.SETTINGS_USAGE


def split_prefix_command(command_line: str) -> PrefixCommand | None:
    if not command_line:
        return None
    command, _, arg = str(command_line or "").partition(" ")
    command = command.lower().strip()
    if not command:
        return None
    return PrefixCommand(command=command, arg=arg.strip())

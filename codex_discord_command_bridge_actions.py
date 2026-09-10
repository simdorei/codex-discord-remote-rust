from __future__ import annotations

from typing import Callable

from codex_discord_command_limits import parse_bounded_int
from codex_discord_command_types import PrefixBridgeAction, RawCommandValue
import codex_discord_settings_commands as settings_commands


def build_list_argv(
    raw_limit: RawCommandValue = "",
    *,
    default: int = 10,
    maximum: int = 30,
) -> list[str]:
    if raw_limit is None or str(raw_limit).strip() == "":
        return ["list", "--db-root", "--limit", "0"]
    limit = parse_bounded_int(raw_limit, default=default, minimum=1, maximum=maximum)
    return ["list", "--db-root", "--limit", str(limit)]


def build_archived_list_argv(
    raw_limit: RawCommandValue = "",
    *,
    default: int = 10,
    maximum: int = 50,
) -> list[str]:
    limit = parse_bounded_int(raw_limit, default=default, minimum=1, maximum=maximum)
    return ["archived_list", "--limit", str(limit)]


def build_open_argv(command: str, ref: str) -> list[str]:
    argv = ["open"]
    if command == "open_abort":
        argv.append("--abort")
    argv.append(ref)
    return argv


def build_status_argv(
    channel_id: int | None,
    ref: str | None,
    *,
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
) -> list[str]:
    argv = ["status"]
    argv.extend(resolve_target_args_func(channel_id, ref or None))
    return argv


def build_stop_argv(
    channel_id: int | None,
    ref: str | None,
    *,
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
) -> list[str]:
    argv = ["stop"]
    argv.extend(resolve_target_args_func(channel_id, ref or None))
    return argv


def build_settings_argv(
    channel_id: int | None,
    raw_arg: str,
    *,
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
) -> PrefixBridgeAction:
    return _prefix_action(
        settings_commands.build_settings_argv(
            channel_id,
            raw_arg,
            resolve_target_args_func=resolve_target_args_func,
        )
    )


def build_settings_values_argv(
    channel_id: int | None,
    ref: str | None,
    *,
    model: str = "",
    effort: str = "",
    speed: str = "",
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
) -> PrefixBridgeAction:
    return _prefix_action(
        settings_commands.build_settings_values_argv(
            channel_id,
            ref,
            model=model,
            effort=effort,
            speed=speed,
            resolve_target_args_func=resolve_target_args_func,
        )
    )


def _prefix_action(action: settings_commands.SettingsBridgeAction) -> PrefixBridgeAction:
    return PrefixBridgeAction(action.argv, action.title, action.usage)


def build_archive_argv(
    channel_id: int | None,
    ref: str | None,
    *,
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
) -> list[str]:
    argv = ["archive"]
    argv.extend(resolve_target_args_func(channel_id, ref or None))
    return argv


def build_prefix_bridge_action(
    command: str,
    arg: str,
    channel_id: int | None,
    *,
    resolve_target_args_func: Callable[[int | None, str | None], list[str]],
    resolve_archive_target_args_func: Callable[[int | None, str | None], list[str]] | None = None,
) -> PrefixBridgeAction | None:
    if command == "list":
        return PrefixBridgeAction(build_list_argv(arg), "List")
    if command in {"archived_list", "archive_list"}:
        return PrefixBridgeAction(build_archived_list_argv(arg), "Archived list")
    if command == "use":
        if not arg:
            return PrefixBridgeAction(None, "Use", "Usage: !use <ref>")
        return PrefixBridgeAction(["use", arg], "Use")
    if command in {"open", "open_abort"}:
        if not arg:
            return PrefixBridgeAction(None, "Open", f"Usage: !{command} <ref>")
        return PrefixBridgeAction(build_open_argv(command, arg), "Open")
    if command == "status":
        return PrefixBridgeAction(
            build_status_argv(
                channel_id,
                arg or None,
                resolve_target_args_func=resolve_target_args_func,
            ),
            "Status",
        )
    if command == "stop":
        return PrefixBridgeAction(
            build_stop_argv(
                channel_id,
                arg or None,
                resolve_target_args_func=resolve_target_args_func,
            ),
            "Stop",
        )
    if command in {"settings", "setting"}:
        return build_settings_argv(
            channel_id,
            arg,
            resolve_target_args_func=resolve_target_args_func,
        )
    if command == "discover_codex":
        return PrefixBridgeAction(["discover_codex"], "Codex path")
    if command == "restart_codex":
        return PrefixBridgeAction(["restart_codex"], "Codex restart")
    if command == "archive":
        archive_target_args_func = resolve_archive_target_args_func or resolve_target_args_func
        return PrefixBridgeAction(
            build_archive_argv(
                channel_id,
                arg or None,
                resolve_target_args_func=archive_target_args_func,
            ),
            "Archive",
        )
    if command == "confirm_delete_archive":
        if not arg:
            return PrefixBridgeAction(None, "Delete archive", "Usage: !confirm_delete_archive <ref>")
        return PrefixBridgeAction(["delete_archive", "--confirm", arg], "Delete archive")
    return None

# pyright: reportPrivateUsage=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse

from codex_desktop_bridge_cli_action_parsers import (
    add_desktop_lifecycle_parsers,
    add_optional_thread_ref,
    add_thread_action_parsers,
)
from codex_desktop_bridge_cli_ask import add_ask_parser
from codex_desktop_bridge_cli_types import BridgeCommandHandlers


def build_parser(handlers: BridgeCommandHandlers) -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Bridge to the current Codex Desktop thread without using Codex CLI.",
    )
    common_parser = argparse.ArgumentParser(add_help=False)
    _add_common_thread_options(common_parser)
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser] = parser.add_subparsers(
        dest="command",
        required=True,
    )

    _add_list_parser(subparsers, common_parser, handlers)
    _add_settings_parsers(subparsers, common_parser, handlers)
    _add_archived_list_parser(subparsers, handlers)
    _add_status_parser(subparsers, common_parser, handlers)
    _add_doctor_parser(subparsers, common_parser, handlers)
    add_desktop_lifecycle_parsers(subparsers, common_parser, handlers)
    add_thread_action_parsers(subparsers, common_parser, handlers)
    add_ask_parser(subparsers, common_parser, handlers.command_ask)
    return parser


def _add_common_thread_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--thread-id", default=None, help="Target a specific thread id.")
    parser.add_argument("--cwd", default=None, help="Prefer the newest thread for this workspace path.")


def _add_settings_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    settings_parser = subparsers.add_parser(
        "settings",
        help="Update model, reasoning effort, or speed tier for a thread.",
        parents=[common_parser],
    )
    add_optional_thread_ref(settings_parser)
    settings_parser.add_argument("--model")
    settings_parser.add_argument("--reasoning")
    settings_parser.add_argument("--speed")
    settings_parser.set_defaults(func=handlers.command_settings)

    settings_options_parser = subparsers.add_parser(
        "settings_options",
        help="Show app-provided model, reasoning effort, and speed options.",
    )
    field_choices = ("all", "model", "effort", "reasoning", "speed")
    settings_options_parser.add_argument("--field", choices=field_choices, default="all")
    settings_options_parser.set_defaults(func=handlers.command_settings_options)


def _add_list_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    list_parser = subparsers.add_parser(
        "list",
        help="List recent local Codex Desktop threads.",
        parents=[common_parser],
    )
    list_parser.add_argument("--limit", type=int, default=10)
    list_parser.add_argument("--db-root", action="store_true")
    list_parser.set_defaults(func=handlers.command_list)


def _add_archived_list_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    handlers: BridgeCommandHandlers,
) -> None:
    archived_list_parser = subparsers.add_parser(
        "archived_list",
        help="List archived local Codex Desktop threads.",
    )
    archived_list_parser.add_argument("--limit", type=int, default=10)
    archived_list_parser.set_defaults(func=handlers.command_archived_list)


def _add_status_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    status_parser = subparsers.add_parser(
        "status",
        help="Show the selected thread and last messages.",
        parents=[common_parser],
    )
    status_parser.set_defaults(func=handlers.command_status)


def _add_doctor_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    doctor_parser = subparsers.add_parser(
        "doctor",
        help="Diagnose Codex Desktop bridge environment and detection state.",
        parents=[common_parser],
    )
    doctor_parser.add_argument(
        "--limit",
        type=int,
        default=5,
        help="Max busy threads to print.",
    )
    doctor_parser.set_defaults(func=handlers.command_doctor)

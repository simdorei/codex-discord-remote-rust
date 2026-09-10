from __future__ import annotations

import argparse
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, runtime_checkable

CommandHandler = Callable[[argparse.Namespace], int]


@runtime_checkable
class CommandFunc(Protocol):
    def __call__(self, args: argparse.Namespace) -> int: ...


def arg_text(args: argparse.Namespace, name: str, default: str = "") -> str:
    return str(getattr(args, name, default) or default)


def arg_optional_text(args: argparse.Namespace, name: str) -> str | None:
    text = arg_text(args, name).strip()
    return text or None


def arg_bool(args: argparse.Namespace, name: str) -> bool:
    return bool(getattr(args, name, False))


def arg_int(args: argparse.Namespace, name: str, default: int = 0) -> int:
    return int(arg_text(args, name, str(default)))


def arg_float(args: argparse.Namespace, name: str, default: float = 0.0) -> float:
    return float(arg_text(args, name, str(default)))


def require_command_func(args: argparse.Namespace, parser: argparse.ArgumentParser) -> CommandFunc:
    func = getattr(args, "func", None)
    if not isinstance(func, CommandFunc):
        parser.error("missing command handler")
    return func


@dataclass(frozen=True, slots=True)
class BridgeCommandHandlers:
    command_list: CommandHandler
    command_settings: CommandHandler
    command_settings_options: CommandHandler
    command_archived_list: CommandHandler
    command_status: CommandHandler
    command_doctor: CommandHandler
    command_discover_codex: CommandHandler
    command_restart_codex: CommandHandler
    command_focus: CommandHandler
    command_new: CommandHandler
    command_archive: CommandHandler
    command_delete_archive: CommandHandler
    command_use: CommandHandler
    command_approval_reply: CommandHandler
    command_tail: CommandHandler
    command_open: CommandHandler
    command_stop: CommandHandler
    command_ask: CommandHandler

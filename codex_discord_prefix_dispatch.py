from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Final, Generic, Protocol, TypeVar

import codex_discord_commands as discord_commands
import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_dispatch_factory as discord_prefix_dispatch_factory
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_resume_command as discord_prefix_resume_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command

HELP_COMMANDS: Final[frozenset[str]] = frozenset({"help", "start"})


class PrefixDispatchChannel(Protocol):
    @property
    def id(self) -> int:
        ...


class PrefixDispatchAuthor(Protocol):
    @property
    def id(self) -> int:
        ...


class PrefixDispatchGuild(Protocol):
    @property
    def id(self) -> int:
        ...


class PrefixDispatchMessage(Protocol):
    @property
    def channel(self) -> PrefixDispatchChannel:
        ...

    @property
    def author(self) -> PrefixDispatchAuthor:
        ...

    @property
    def guild(self) -> PrefixDispatchGuild | None:
        ...


class SendChunksFunc(Protocol):
    def __call__(
        self,
        target: PrefixDispatchChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> Awaitable[int]:
        ...


class BuildBridgeActionFunc(Protocol):
    def __call__(
        self,
        command: str,
        arg: str,
        channel_id: int,
    ) -> discord_commands.PrefixBridgeAction | None:
        ...


class RunBridgeActionFunc(Protocol):
    def __call__(
        self,
        target: PrefixDispatchChannel,
        argv: list[str],
        title: str,
    ) -> Awaitable[tuple[int, str]]:
        ...


class BuildDoctorMessageFunc(Protocol):
    def __call__(self, message: PrefixDispatchMessage) -> Awaitable[str]:
        ...


class RunDoctorBridgeFunc(Protocol):
    def __call__(self, target: PrefixDispatchChannel) -> Awaitable[tuple[int, str]]:
        ...


class PrefixHandler(Protocol):
    def __call__(self, command: str, arg: str, message: PrefixDispatchMessage) -> Awaitable[bool]:
        ...


class PrefixDispatchBot(Protocol):
    ...


BotT = TypeVar("BotT", bound=PrefixDispatchBot)


@dataclass(frozen=True, slots=True)
class PrefixDispatchDeps:
    send_chunks: SendChunksFunc
    build_help: Callable[[], str]
    build_bridge_action: BuildBridgeActionFunc
    run_bridge_action: RunBridgeActionFunc
    build_doctor_message: BuildDoctorMessageFunc
    run_doctor_bridge: RunDoctorBridgeFunc
    format_command_label: Callable[[str], str]
    handlers: tuple[PrefixHandler, ...]


@dataclass(frozen=True, slots=True)
class PrefixDispatchFactoryDeps(Generic[BotT]):
    bot: BotT
    send_chunks: SendChunksFunc
    build_help: Callable[[], str]
    build_bridge_action: BuildBridgeActionFunc
    run_bridge_action: RunBridgeActionFunc
    build_doctor_message: BuildDoctorMessageFunc
    run_doctor_bridge: RunDoctorBridgeFunc
    format_command_label: Callable[[str], str]
    make_prefix_steer_deps: Callable[[], discord_prefix_steer_command.PrefixSteerCommandDeps]
    make_prefix_status_deps: Callable[[], discord_prefix_status_commands.PrefixStatusCommandDeps]
    make_prefix_queue_deps: Callable[[], discord_prefix_queue_commands.PrefixQueueCommandDeps]
    make_prefix_resume_deps: Callable[[], discord_prefix_resume_command.PrefixResumeCommandDeps]
    make_prefix_mirror_deps: Callable[[], discord_prefix_mirror_commands.PrefixMirrorCommandDeps]
    make_prefix_approval_deps: Callable[[], discord_prefix_approval_commands.PrefixApprovalCommandDeps]
    make_prefix_archive_deps: Callable[[], discord_prefix_archive_commands.PrefixArchiveCommandDeps]
    make_prefix_qa_deps: Callable[[], discord_prefix_qa_command.PrefixQaCommandDeps[BotT]]
    make_prefix_new_deps: Callable[[], discord_prefix_new_command.PrefixNewCommandDeps]
    make_prefix_prompt_deps: Callable[[], discord_prefix_prompt_commands.PrefixPromptCommandDeps]
    make_prefix_host_deps: Callable[[], discord_prefix_host_commands.PrefixHostCommandDeps]


def build_prefix_dispatch_deps(factory: PrefixDispatchFactoryDeps[BotT]) -> PrefixDispatchDeps:
    return PrefixDispatchDeps(
        send_chunks=factory.send_chunks,
        build_help=factory.build_help,
        build_bridge_action=factory.build_bridge_action,
        run_bridge_action=factory.run_bridge_action,
        build_doctor_message=factory.build_doctor_message,
        run_doctor_bridge=factory.run_doctor_bridge,
        format_command_label=factory.format_command_label,
        handlers=discord_prefix_dispatch_factory.build_prefix_handlers(factory),
    )


async def handle_prefix_command(
    message: PrefixDispatchMessage,
    command_line: str,
    *,
    deps: PrefixDispatchDeps,
) -> None:
    parsed = discord_commands.split_prefix_command(command_line)
    if parsed is None:
        _ = await deps.send_chunks(message.channel, deps.build_help())
        return

    command = parsed.command
    arg = parsed.arg

    if command in HELP_COMMANDS:
        _ = await deps.send_chunks(message.channel, deps.build_help())
        return

    bridge_action = deps.build_bridge_action(command, arg, message.channel.id)
    if bridge_action is not None:
        if bridge_action.usage:
            _ = await deps.send_chunks(
                message.channel,
                bridge_action.usage,
                context="prefix_bridge_usage",
            )
            return
        _ = await deps.run_bridge_action(
            message.channel,
            bridge_action.argv or [],
            bridge_action.title,
        )
        return

    if command == "doctor":
        _ = await deps.send_chunks(message.channel, await deps.build_doctor_message(message))
        _ = await deps.run_doctor_bridge(message.channel)
        return

    for handler in deps.handlers:
        if await handler(command, arg, message):
            return

    _ = await deps.send_chunks(
        message.channel,
        f"Unknown command: !{deps.format_command_label(command)}",
        context="prefix_unknown",
    )

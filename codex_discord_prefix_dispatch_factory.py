from __future__ import annotations

from collections.abc import Awaitable
from typing import Protocol, TypeVar

import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_resume_command as discord_prefix_resume_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command


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


class PrefixHandler(Protocol):
    def __call__(self, command: str, arg: str, message: PrefixDispatchMessage) -> Awaitable[bool]:
        ...


class PrefixDispatchBot(Protocol):
    ...


BotT = TypeVar("BotT", bound=PrefixDispatchBot)


class PrefixDispatchFactoryDeps(Protocol[BotT]):
    @property
    def bot(self) -> BotT:
        ...

    def make_prefix_steer_deps(self) -> discord_prefix_steer_command.PrefixSteerCommandDeps:
        ...

    def make_prefix_status_deps(self) -> discord_prefix_status_commands.PrefixStatusCommandDeps:
        ...

    def make_prefix_queue_deps(self) -> discord_prefix_queue_commands.PrefixQueueCommandDeps:
        ...

    def make_prefix_resume_deps(self) -> discord_prefix_resume_command.PrefixResumeCommandDeps:
        ...

    def make_prefix_mirror_deps(self) -> discord_prefix_mirror_commands.PrefixMirrorCommandDeps:
        ...

    def make_prefix_approval_deps(self) -> discord_prefix_approval_commands.PrefixApprovalCommandDeps:
        ...

    def make_prefix_archive_deps(self) -> discord_prefix_archive_commands.PrefixArchiveCommandDeps:
        ...

    def make_prefix_qa_deps(self) -> discord_prefix_qa_command.PrefixQaCommandDeps[BotT]:
        ...

    def make_prefix_new_deps(self) -> discord_prefix_new_command.PrefixNewCommandDeps:
        ...

    def make_prefix_prompt_deps(self) -> discord_prefix_prompt_commands.PrefixPromptCommandDeps:
        ...

    def make_prefix_host_deps(self) -> discord_prefix_host_commands.PrefixHostCommandDeps:
        ...


def build_prefix_handlers(factory: PrefixDispatchFactoryDeps[BotT]) -> tuple[PrefixHandler, ...]:
    async def handle_resume(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_resume_command.handle_prefix_resume_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_resume_deps(),
        )

    async def handle_steer(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_steer_command.handle_prefix_steer_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_steer_deps(),
        )

    async def handle_status(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_status_commands.handle_prefix_status_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_status_deps(),
        )

    async def handle_queue(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_queue_commands.handle_prefix_queue_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_queue_deps(),
        )

    async def handle_mirror(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_mirror_commands.handle_prefix_mirror_command(
            command,
            arg,
            message,
            factory.bot,
            deps=factory.make_prefix_mirror_deps(),
        )

    async def handle_approval(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_approval_commands.handle_prefix_approval_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_approval_deps(),
        )

    async def handle_archive(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_archive_commands.handle_prefix_archive_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_archive_deps(),
        )

    async def handle_qa(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_qa_command.handle_prefix_qa_command(
            command,
            arg,
            message,
            factory.bot,
            deps=factory.make_prefix_qa_deps(),
        )

    async def handle_new(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_new_command.handle_prefix_new_command(
            command,
            arg,
            message,
            factory.bot,
            deps=factory.make_prefix_new_deps(),
        )

    async def handle_prompt(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_prompt_commands.handle_prefix_prompt_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_prompt_deps(),
        )

    async def handle_host(command: str, arg: str, message: PrefixDispatchMessage) -> bool:
        return await discord_prefix_host_commands.handle_prefix_host_command(
            command,
            arg,
            message,
            deps=factory.make_prefix_host_deps(),
        )

    return (
        handle_host,
        handle_resume,
        handle_steer,
        handle_status,
        handle_queue,
        handle_mirror,
        handle_approval,
        handle_archive,
        handle_qa,
        handle_new,
        handle_prompt,
    )

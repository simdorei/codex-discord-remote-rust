from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

import codex_discord_commands as discord_commands
import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_dispatch as discord_prefix_dispatch
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_resume_command as discord_prefix_resume_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command

BotT = TypeVar("BotT", bound=discord_prefix_dispatch.PrefixDispatchBot)
BotContraT = TypeVar("BotContraT", bound=discord_prefix_dispatch.PrefixDispatchBot, contravariant=True)
MessageableT = TypeVar("MessageableT")
MessageableContraT = TypeVar("MessageableContraT", contravariant=True)
HistoryChannelT = TypeVar("HistoryChannelT")
HistoryChannelContraT = TypeVar("HistoryChannelContraT", contravariant=True)


class BridgeRunner(Protocol[MessageableContraT, BotContraT]):
    def __call__(
        self,
        target: MessageableContraT,
        argv: list[str],
        title: str,
        failure_title: str | None = None,
        archive_cleanup_owner: BotContraT | None = None,
    ) -> Awaitable[tuple[int, str]]: ...


class DoctorMessageBuilder(Protocol[BotContraT, HistoryChannelContraT]):
    def __call__(
        self,
        bot: BotContraT,
        channel_id: int | None,
        channel: HistoryChannelContraT | None,
    ) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class PrefixDispatchRuntimeDeps(Generic[BotT, MessageableT, HistoryChannelT]):
    send_prefix_chunks: discord_prefix_dispatch.SendChunksFunc
    build_help: Callable[[], str]
    resolve_thread_target_args: Callable[[int | None, str | None], list[str]]
    resolve_archive_target_args: Callable[[int | None, str | None], list[str]]
    run_bridge_and_send: BridgeRunner[MessageableT, BotT]
    require_messageable: Callable[[discord_prefix_dispatch.PrefixDispatchChannel], MessageableT]
    build_doctor_message_with_history: DoctorMessageBuilder[BotT, HistoryChannelT]
    require_history_channel: Callable[[discord_prefix_dispatch.PrefixDispatchChannel], HistoryChannelT]
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


def make_prefix_dispatch_deps(
    bot: BotT,
    *,
    deps: PrefixDispatchRuntimeDeps[BotT, MessageableT, HistoryChannelT],
) -> discord_prefix_dispatch.PrefixDispatchDeps:
    def build_bridge_action(
        command: str,
        arg: str,
        channel_id: int,
    ) -> discord_commands.PrefixBridgeAction | None:
        return discord_commands.build_prefix_bridge_action(
            command,
            arg,
            channel_id,
            resolve_target_args_func=deps.resolve_thread_target_args,
            resolve_archive_target_args_func=deps.resolve_archive_target_args,
        )

    async def run_bridge_action(
        target: discord_prefix_dispatch.PrefixDispatchChannel,
        argv: list[str],
        title: str,
    ) -> tuple[int, str]:
        return await deps.run_bridge_and_send(
            deps.require_messageable(target),
            argv,
            title,
            archive_cleanup_owner=bot,
        )

    async def build_doctor_message(message: discord_prefix_dispatch.PrefixDispatchMessage) -> str:
        return await deps.build_doctor_message_with_history(
            bot,
            message.channel.id,
            deps.require_history_channel(message.channel),
        )

    async def run_doctor_bridge(
        target: discord_prefix_dispatch.PrefixDispatchChannel,
    ) -> tuple[int, str]:
        return await deps.run_bridge_and_send(deps.require_messageable(target), ["doctor"], "Doctor")

    return discord_prefix_dispatch.build_prefix_dispatch_deps(
        discord_prefix_dispatch.PrefixDispatchFactoryDeps(
            bot=bot,
            send_chunks=deps.send_prefix_chunks,
            build_help=deps.build_help,
            build_bridge_action=build_bridge_action,
            run_bridge_action=run_bridge_action,
            build_doctor_message=build_doctor_message,
            run_doctor_bridge=run_doctor_bridge,
            format_command_label=deps.format_command_label,
            make_prefix_steer_deps=deps.make_prefix_steer_deps,
            make_prefix_status_deps=deps.make_prefix_status_deps,
            make_prefix_queue_deps=deps.make_prefix_queue_deps,
            make_prefix_resume_deps=deps.make_prefix_resume_deps,
            make_prefix_mirror_deps=deps.make_prefix_mirror_deps,
            make_prefix_approval_deps=deps.make_prefix_approval_deps,
            make_prefix_archive_deps=deps.make_prefix_archive_deps,
            make_prefix_qa_deps=deps.make_prefix_qa_deps,
            make_prefix_new_deps=deps.make_prefix_new_deps,
            make_prefix_prompt_deps=deps.make_prefix_prompt_deps,
            make_prefix_host_deps=deps.make_prefix_host_deps,
        )
    )

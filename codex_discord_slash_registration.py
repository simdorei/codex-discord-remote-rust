from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeAlias, TypeVar, cast

import codex_discord_slash_commands as discord_slash_commands
import codex_discord_slash_prompt_commands as discord_slash_prompt_commands
import codex_discord_slash_runtime_commands as discord_slash_runtime_commands

BotT = TypeVar("BotT")
BotT_contra = TypeVar("BotT_contra", contravariant=True)
ResolvedInteractionT = TypeVar("ResolvedInteractionT")
ResolvedInteractionT_co = TypeVar("ResolvedInteractionT_co", covariant=True)
ResolvedInteractionT_contra = TypeVar("ResolvedInteractionT_contra", contravariant=True)
SlashInteractionCandidate: TypeAlias = (
    discord_slash_commands.BasicSlashInteraction
    | discord_slash_runtime_commands.RuntimeSlashInteraction
    | discord_slash_prompt_commands.PromptSlashInteraction
)


class DiscordInteractionResolver(Protocol[ResolvedInteractionT_co]):
    def __call__(self, interaction: SlashInteractionCandidate) -> ResolvedInteractionT_co: ...


class InteractionAllowedChecker(Protocol[BotT_contra, ResolvedInteractionT_contra]):
    def __call__(self, bot: BotT_contra, interaction: ResolvedInteractionT_contra) -> bool: ...


class DiscordInteractionChunksSender(Protocol[ResolvedInteractionT_contra]):
    def __call__(
        self,
        interaction: ResolvedInteractionT_contra,
        text: str,
        *,
        title: str,
    ) -> Awaitable[None]: ...


class DiscordInteractionBridgeRunner(Protocol[ResolvedInteractionT_contra]):
    def __call__(
        self,
        interaction: ResolvedInteractionT_contra,
        argv: list[str],
        title: str,
    ) -> Awaitable[tuple[int, str]]: ...


class DiscordInteractionResponseSender(Protocol[ResolvedInteractionT_contra]):
    def __call__(
        self,
        interaction: ResolvedInteractionT_contra,
        content: str,
        *,
        ephemeral: bool = False,
        context: str = "interaction_response",
    ) -> Awaitable[None]: ...


class SlashNewHandler(Protocol[BotT_contra, ResolvedInteractionT_contra]):
    def __call__(
        self,
        bot: BotT_contra,
        interaction: ResolvedInteractionT_contra,
        prompt: str,
    ) -> Awaitable[None]: ...


class SlashPromptHandler(Protocol[ResolvedInteractionT_contra]):
    def __call__(self, interaction: ResolvedInteractionT_contra, prompt: str) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class SlashRegistrationDeps(Generic[BotT, ResolvedInteractionT]):
    check_interaction_allowed: InteractionAllowedChecker[BotT, ResolvedInteractionT]
    require_discord_interaction: DiscordInteractionResolver[ResolvedInteractionT]
    send_interaction_not_allowed: Callable[[ResolvedInteractionT], Awaitable[None]]
    send_interaction_chunks: DiscordInteractionChunksSender[ResolvedInteractionT]
    run_interaction_bridge_and_send: DiscordInteractionBridgeRunner[ResolvedInteractionT]
    send_interaction_response_tracked: DiscordInteractionResponseSender[ResolvedInteractionT]
    build_help: Callable[[], str]
    build_where_message: Callable[[int | None], str]
    build_context_message: discord_slash_commands.ContextMessageBuilder
    build_context_refresh_message: discord_slash_commands.ContextRefreshMessageBuilder
    build_weekly_usage_message: discord_slash_commands.WeeklyUsageMessageBuilder
    clamp_context_refresh_limit: Callable[[int], int]
    resolve_discord_thread_target_args: Callable[[int | None, str | None], list[str]]
    load_settings_model_catalog: discord_slash_commands.SettingsModelCatalogLoader
    build_mirror_check: Callable[[], str]
    build_runtime_discord_doctor_message: discord_slash_runtime_commands.RuntimeDoctorMessageBuilder
    build_runners_message: Callable[[], Awaitable[str]]
    retract_queued_ask_for_request: discord_slash_runtime_commands.RuntimeQueueRetractor
    refresh_runtime_discord_bridge_session: discord_slash_runtime_commands.RuntimeBridgeSessionRefresher
    discord_qa_commands_enabled: Callable[[], bool]
    run_runtime_discord_button_qa: discord_slash_runtime_commands.RuntimeButtonQaRunner
    handle_slash_new: SlashNewHandler[BotT, ResolvedInteractionT]
    handle_slash_ask: SlashPromptHandler[ResolvedInteractionT]
    handle_slash_interview: SlashPromptHandler[ResolvedInteractionT]
    log_line: Callable[[str], None]


def register_commands(bot: BotT, deps: SlashRegistrationDeps[BotT, ResolvedInteractionT]) -> None:
    slash_bot = cast(discord_slash_commands.SlashCommandBot, bot)

    def basic_allowed(interaction: discord_slash_commands.BasicSlashInteraction) -> bool:
        return deps.check_interaction_allowed(bot, deps.require_discord_interaction(interaction))

    async def send_basic_not_allowed(interaction: discord_slash_commands.SlashInteraction) -> None:
        await deps.send_interaction_not_allowed(deps.require_discord_interaction(interaction))

    async def send_basic_chunks(
        interaction: discord_slash_commands.SlashInteraction,
        text: str,
        *,
        title: str,
    ) -> None:
        await deps.send_interaction_chunks(deps.require_discord_interaction(interaction), text, title=title)

    async def run_basic_bridge(
        interaction: discord_slash_commands.SlashInteraction,
        argv: list[str],
        title: str,
    ) -> tuple[int, str]:
        return await deps.run_interaction_bridge_and_send(
            deps.require_discord_interaction(interaction),
            argv,
            title,
        )

    discord_slash_commands.register_basic_slash_commands(
        slash_bot,
        discord_slash_commands.BasicSlashCommandDeps(
            check_allowed=basic_allowed,
            send_not_allowed=send_basic_not_allowed,
            send_chunks=send_basic_chunks,
            run_bridge=run_basic_bridge,
            build_help=deps.build_help,
            build_where=deps.build_where_message,
            build_context=deps.build_context_message,
            build_context_refresh=deps.build_context_refresh_message,
            build_weekly_usage=deps.build_weekly_usage_message,
            clamp_context_refresh_limit=deps.clamp_context_refresh_limit,
            resolve_target_args=deps.resolve_discord_thread_target_args,
            load_settings_model_catalog=deps.load_settings_model_catalog,
        ),
    )

    def runtime_allowed(interaction: discord_slash_runtime_commands.RuntimeSlashInteraction) -> bool:
        return deps.check_interaction_allowed(bot, deps.require_discord_interaction(interaction))

    async def send_runtime_response(
        interaction: discord_slash_runtime_commands.RuntimeSlashInteraction,
        content: str,
        *,
        ephemeral: bool = False,
        context: str = "interaction_response",
    ) -> None:
        await deps.send_interaction_response_tracked(
            deps.require_discord_interaction(interaction),
            content,
            ephemeral=ephemeral,
            context=context,
        )

    async def run_runtime_mirror_check() -> str:
        return await asyncio.to_thread(deps.build_mirror_check)

    discord_slash_runtime_commands.register_runtime_slash_commands(
        slash_bot,
        discord_slash_runtime_commands.RuntimeSlashCommandDeps(
            check_allowed=runtime_allowed,
            send_not_allowed=send_basic_not_allowed,
            send_chunks=send_basic_chunks,
            run_bridge=run_basic_bridge,
            build_doctor=deps.build_runtime_discord_doctor_message,
            build_runners=deps.build_runners_message,
            retract_queued_ask=deps.retract_queued_ask_for_request,
            run_mirror_check=run_runtime_mirror_check,
            refresh_bridge_session=deps.refresh_runtime_discord_bridge_session,
            qa_commands_enabled=deps.discord_qa_commands_enabled,
            send_response=send_runtime_response,
            run_button_qa=deps.run_runtime_discord_button_qa,
            log_line=deps.log_line,
        ),
    )

    def prompt_allowed(interaction: discord_slash_prompt_commands.PromptSlashInteraction) -> bool:
        return deps.check_interaction_allowed(bot, deps.require_discord_interaction(interaction))

    async def send_prompt_not_allowed(
        interaction: discord_slash_prompt_commands.PromptSlashInteraction,
    ) -> None:
        await deps.send_interaction_not_allowed(deps.require_discord_interaction(interaction))

    async def handle_prompt_new(
        interaction: discord_slash_prompt_commands.PromptSlashInteraction,
        prompt: str,
    ) -> None:
        await deps.handle_slash_new(bot, deps.require_discord_interaction(interaction), prompt)

    async def handle_prompt_ask(
        interaction: discord_slash_prompt_commands.PromptSlashInteraction,
        prompt: str,
    ) -> None:
        await deps.handle_slash_ask(deps.require_discord_interaction(interaction), prompt)

    async def handle_prompt_interview(
        interaction: discord_slash_prompt_commands.PromptSlashInteraction,
        prompt: str,
    ) -> None:
        await deps.handle_slash_interview(deps.require_discord_interaction(interaction), prompt)

    discord_slash_prompt_commands.register_prompt_slash_commands(
        slash_bot,
        discord_slash_prompt_commands.PromptSlashCommandDeps(
            check_allowed=prompt_allowed,
            send_not_allowed=send_prompt_not_allowed,
            handle_new=handle_prompt_new,
            handle_ask=handle_prompt_ask,
            handle_interview=handle_prompt_interview,
        ),
    )

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_discord_persistent_interactions as discord_persistent_interactions
import codex_discord_unhandled_component_report as discord_unhandled_component_report

DiscordInteraction: TypeAlias = discord_unhandled_component_report.UnhandledComponentInteraction


class InteractionComponentsClearer(Protocol):
    def __call__(self, interaction: DiscordInteraction, *, context: str) -> Awaitable[None]: ...


class InteractionResponseSender(Protocol):
    def __call__(
        self,
        interaction: DiscordInteraction,
        content: str,
        *,
        ephemeral: bool = False,
        context: str,
    ) -> Awaitable[None]: ...


class InteractionFollowupChunksSender(Protocol):
    def __call__(
        self,
        interaction: DiscordInteraction,
        text: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
    ) -> Awaitable[None]: ...


class ApprovalResultStreamer(Protocol):
    def __call__(
        self,
        interaction: DiscordInteraction,
        watch_result: discord_persistent_interactions.ApprovalWatchResult,
        target_thread_id: str,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class BotPersistentComponentRuntime:
    module: ModuleType

    def adapt_persistent_interaction(
        self,
        interaction: DiscordInteraction,
    ) -> discord_persistent_interactions.PersistentInteraction:
        return cast(discord_persistent_interactions.PersistentInteraction, cast(object, interaction))

    def require_discord_persistent_interaction(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
    ) -> DiscordInteraction:
        return cast(DiscordInteraction, cast(object, interaction))

    def claim_persistent_component_for_persistent_interaction(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
        custom_id: str,
    ) -> bool:
        claim_component = cast(
            Callable[[DiscordInteraction, str], bool],
            getattr(self.module, "claim_persistent_component_interaction"),
        )
        return claim_component(self.require_discord_persistent_interaction(interaction), custom_id)

    async def clear_persistent_interaction_components(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
        *,
        context: str,
    ) -> None:
        clear_components = cast(
            InteractionComponentsClearer,
            getattr(self.module, "clear_interaction_message_components"),
        )
        await clear_components(self.require_discord_persistent_interaction(interaction), context=context)

    async def send_persistent_interaction_response(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> None:
        send_response = cast(InteractionResponseSender, getattr(self.module, "send_interaction_response_tracked"))
        await send_response(
            self.require_discord_persistent_interaction(interaction),
            content,
            ephemeral=ephemeral,
            context=context,
        )

    async def send_persistent_followup_chunks(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
        text: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
    ) -> None:
        send_chunks = cast(InteractionFollowupChunksSender, getattr(self.module, "send_followup_chunks"))
        await send_chunks(
            self.require_discord_persistent_interaction(interaction),
            text,
            title=title,
            exit_code=exit_code,
            log_prefix=log_prefix,
        )

    async def stream_post_approval_result_for_persistent_interaction(
        self,
        interaction: discord_persistent_interactions.PersistentInteraction,
        watch_result: discord_persistent_interactions.ApprovalWatchResult,
        target_thread_id: str,
    ) -> bool:
        stream_result = cast(
            ApprovalResultStreamer,
            getattr(self.module, "stream_post_approval_result_for_interaction"),
        )
        return await stream_result(
            self.require_discord_persistent_interaction(interaction),
            watch_result,
            target_thread_id,
        )

    async def report_unhandled_component_interaction(
        self,
        interaction: DiscordInteraction,
        *,
        delay_sec: float = 0.75,
    ) -> None:
        handlers = cast(
            tuple[discord_unhandled_component_report.UnhandledComponentHandler, ...],
            (
                cast(
                    discord_unhandled_component_report.UnhandledComponentHandler,
                    getattr(self.module, "handle_persistent_approval_interaction"),
                ),
                cast(
                    discord_unhandled_component_report.UnhandledComponentHandler,
                    getattr(self.module, "handle_persistent_input_choice_interaction"),
                ),
                cast(
                    discord_unhandled_component_report.UnhandledComponentHandler,
                    getattr(self.module, "handle_persistent_busy_choice_interaction"),
                ),
            ),
        )
        await discord_unhandled_component_report.report_discord_unhandled_component_interaction(
            interaction,
            delay_sec=delay_sec,
            persistent_handlers=handlers,
            clear_components=cast(
                discord_unhandled_component_report.UnhandledComponentClearer,
                getattr(self.module, "clear_interaction_message_components"),
            ),
            send_response=cast(
                discord_unhandled_component_report.UnhandledComponentResponder,
                getattr(self.module, "send_interaction_response_tracked"),
            ),
            is_already_acknowledged=cast(
                Callable[[BaseException], bool],
                getattr(self.module, "is_interaction_already_acknowledged_error"),
            ),
            delivery_exceptions=cast(
                discord_unhandled_component_report.DeliveryExceptions,
                getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"),
            ),
            log=cast(Callable[[str], None], getattr(self.module, "log_line")),
        )

    async def handle_persistent_approval_interaction(
        self,
        interaction: DiscordInteraction,
        custom_id: str,
        *,
        approval_submitter: discord_persistent_interactions.Submitter | None = None,
    ) -> bool:
        submitter = approval_submitter
        if submitter is None:
            submitter = cast(discord_persistent_interactions.Submitter, getattr(self.module, "submit_approval_reply"))
        return await discord_persistent_interactions.handle_persistent_approval_interaction(
            self.adapt_persistent_interaction(interaction),
            custom_id,
            approval_submitter=submitter,
            deps=discord_persistent_interactions.PersistentApprovalDeps(
                is_user_allowed=cast(
                    Callable[[int], bool],
                    getattr(cast(ModuleType, getattr(self.module, "discord_interaction_gate")), "is_discord_user_allowed"),
                ),
                claim_component=self.claim_persistent_component_for_persistent_interaction,
                clear_components=self.clear_persistent_interaction_components,
                send_response=self.send_persistent_interaction_response,
                send_followup_chunks=self.send_persistent_followup_chunks,
                make_watch_result=cast(
                    Callable[[str], discord_persistent_interactions.ApprovalWatchResult],
                    getattr(self.module, "make_post_approval_watch_result"),
                ),
                stream_post_approval_result=self.stream_post_approval_result_for_persistent_interaction,
                format_log_text_len=cast(
                    discord_persistent_interactions.TextLenFormatter,
                    getattr(self.module, "format_log_text_len_as_text"),
                ),
                log=cast(discord_persistent_interactions.LogFunc, getattr(self.module, "log_line")),
            ),
        )

    async def handle_persistent_input_choice_interaction(
        self,
        interaction: DiscordInteraction,
        custom_id: str,
        *,
        input_submitter: discord_persistent_interactions.Submitter | None = None,
    ) -> bool:
        submitter = input_submitter
        if submitter is None:
            submitter = cast(discord_persistent_interactions.Submitter, getattr(self.module, "submit_input_reply"))
        return await discord_persistent_interactions.handle_persistent_input_choice_interaction(
            self.adapt_persistent_interaction(interaction),
            custom_id,
            input_submitter=submitter,
            deps=discord_persistent_interactions.PersistentInputChoiceDeps(
                is_user_allowed=cast(
                    Callable[[int], bool],
                    getattr(cast(ModuleType, getattr(self.module, "discord_interaction_gate")), "is_discord_user_allowed"),
                ),
                claim_component=self.claim_persistent_component_for_persistent_interaction,
                clear_components=self.clear_persistent_interaction_components,
                send_response=self.send_persistent_interaction_response,
                send_followup_chunks=self.send_persistent_followup_chunks,
                format_log_text_len=cast(
                    discord_persistent_interactions.TextLenFormatter,
                    getattr(self.module, "format_log_text_len_as_text"),
                ),
                log=cast(discord_persistent_interactions.LogFunc, getattr(self.module, "log_line")),
            ),
        )

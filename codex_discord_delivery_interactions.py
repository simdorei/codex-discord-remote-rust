from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

import discord

from codex_discord_delivery_state import (
    DiscordIdValue,
    DiscordDeliveryState,
    InteractionCommandLike,
    InteractionCommandSource,
    InteractionLike,
    InteractionResponse,
    LogFunc,
    begin_discord_delivery,
    end_discord_delivery,
    get_interaction_command_name,
)
from codex_discord_text import format_discord_command_label, format_log_text_len, split_message

__all__ = [
    "adapt_discord_interaction",
    "send_direct_followup",
    "send_followup_chunks",
    "send_interaction_not_allowed",
    "send_interaction_response_tracked",
]


class FollowupView(Protocol):
    pass


class InteractionFollowup(Protocol):
    async def send(self, content: str, **kwargs: bool | FollowupView) -> None: ...


class FollowupInteraction(InteractionCommandSource, Protocol):
    @property
    def channel_id(self) -> DiscordIdValue: ...

    @property
    def followup(self) -> InteractionFollowup: ...


@dataclass(frozen=True, slots=True)
class DiscordInteractionCommandAdapter:
    name: str


@dataclass(frozen=True, slots=True)
class DiscordInteractionResponseAdapter:
    interaction: discord.Interaction

    async def send_message(self, content: str, *, ephemeral: bool = False) -> None:
        _ = await self.interaction.response.send_message(content, ephemeral=ephemeral)


@dataclass(frozen=True, slots=True)
class DiscordInteractionAdapter:
    interaction: discord.Interaction

    @property
    def command(self) -> InteractionCommandLike | None:
        command = self.interaction.command
        return None if command is None else DiscordInteractionCommandAdapter(str(command.name or "-"))

    @property
    def response(self) -> InteractionResponse:
        return DiscordInteractionResponseAdapter(self.interaction)

    @property
    def channel_id(self) -> DiscordIdValue:
        return self.interaction.channel_id


def adapt_discord_interaction(interaction: discord.Interaction) -> InteractionLike:
    return DiscordInteractionAdapter(interaction)


async def send_followup_chunks(
    state: DiscordDeliveryState,
    interaction: FollowupInteraction,
    text: str,
    *,
    log_func: LogFunc,
    title: str,
    exit_code: int | None = None,
    log_prefix: str = "followup_response",
    ephemeral: bool = False,
    allow_during_stop: bool = False,
) -> None:
    chunks = split_message(text)
    command_name = get_interaction_command_name(interaction)
    exit_part = "-" if exit_code is None else str(exit_code)
    log_func(
        f"{log_prefix}_start command={command_name} title={title!r} "
        + f"exit={exit_part} chunks={len(chunks)} channel={interaction.channel_id}"
    )
    delivery_token = begin_discord_delivery(
        state,
        f"followup:{command_name}:{interaction.channel_id}:{log_prefix}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    sent_count = 0
    try:
        for chunk in chunks:
            if ephemeral:
                await interaction.followup.send(chunk, ephemeral=True)
            else:
                await interaction.followup.send(chunk)
            sent_count += 1
    except (discord.DiscordException, OSError, RuntimeError) as exc:
        log_func(
            f"{log_prefix}_failed command={command_name} title={title!r} "
            + f"exit={exit_part} sent={sent_count} chunks={len(chunks)} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        raise
    finally:
        end_discord_delivery(state, delivery_token)
    log_func(
        f"{log_prefix}_sent command={command_name} title={title!r} "
        + f"exit={exit_part} chunks={len(chunks)}"
    )


async def send_direct_followup(
    state: DiscordDeliveryState,
    interaction: FollowupInteraction,
    content: str,
    *,
    log_func: LogFunc,
    ephemeral: bool = False,
    view: FollowupView | None = None,
    log_prefix: str = "direct_followup",
    context: str = "",
    allow_during_stop: bool = False,
) -> None:
    command_name = get_interaction_command_name(interaction)
    safe_context = format_discord_command_label(context, limit=80)
    has_view = view is not None
    channel_id = interaction.channel_id or "-"
    delivery_token = begin_discord_delivery(
        state,
        f"direct_followup:{command_name}:{channel_id}:{safe_context or '-'}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    try:
        if has_view:
            await interaction.followup.send(content, view=view, ephemeral=ephemeral)
        elif ephemeral:
            await interaction.followup.send(content, ephemeral=True)
        else:
            await interaction.followup.send(content)
        log_func(
            f"{log_prefix}_sent command={command_name} context={safe_context or '-'} "
            + f"has_view={has_view} ephemeral={ephemeral} content_len={format_log_text_len(content)}"
        )
    except (discord.DiscordException, OSError, RuntimeError) as exc:
        log_func(
            f"{log_prefix}_failed command={command_name} context={safe_context or '-'} "
            + f"has_view={has_view} ephemeral={ephemeral} content_len={format_log_text_len(content)} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        raise
    finally:
        end_discord_delivery(state, delivery_token)


async def send_interaction_response_tracked(
    state: DiscordDeliveryState,
    interaction: InteractionLike,
    content: str,
    *,
    log_func: LogFunc,
    ephemeral: bool = False,
    context: str = "interaction_response",
    allow_during_stop: bool = False,
) -> None:
    command_name = get_interaction_command_name(interaction)
    safe_context = format_discord_command_label(context, limit=120)
    channel_id = getattr(interaction, "channel_id", "-") or "-"
    delivery_token = begin_discord_delivery(
        state,
        f"response:{command_name}:{channel_id}:{safe_context}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    try:
        _ = await interaction.response.send_message(content, ephemeral=ephemeral)
        log_func(
            f"interaction_response_sent command={command_name} context={safe_context or '-'} "
            + f"channel={channel_id} ephemeral={ephemeral} text_len={format_log_text_len(content)}"
        )
    except (discord.DiscordException, OSError, RuntimeError) as exc:
        log_func(
            f"interaction_response_failed command={command_name} context={safe_context or '-'} "
            + f"channel={channel_id} ephemeral={ephemeral} text_len={format_log_text_len(content)} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        raise
    finally:
        end_discord_delivery(state, delivery_token)


async def send_interaction_not_allowed(
    state: DiscordDeliveryState,
    interaction: InteractionLike,
    *,
    log_func: LogFunc,
) -> None:
    await send_interaction_response_tracked(
        state,
        interaction,
        "This channel/user is not allowed.",
        log_func=log_func,
        ephemeral=True,
        context="interaction_not_allowed",
    )

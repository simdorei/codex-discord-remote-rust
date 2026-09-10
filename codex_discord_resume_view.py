from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, TypeVar, override

import discord

ChannelContraT = TypeVar("ChannelContraT", contravariant=True)


class ResumeFailureMessageSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        view: discord.ui.View,
        context: str,
    ) -> Awaitable[object]: ...


class RecoverResidentThreadFunc(Protocol):
    def __call__(self, channel_id: int, ref: str | None) -> Awaitable[str]: ...


class InteractionResponseSender(Protocol):
    def __call__(
        self,
        interaction: discord.Interaction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


class DirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: discord.Interaction,
        content: str,
        *,
        ephemeral: bool,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class ResumeActionDeps:
    recover_resident_thread_for_request: RecoverResidentThreadFunc
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class ResumeViewDeps(ResumeActionDeps):
    is_user_allowed: Callable[[int | None], bool]
    send_interaction_response: InteractionResponseSender
    send_direct_followup: DirectFollowupSender


async def build_resume_button_response(
    channel_id: int | None,
    target_thread_id: str,
    *,
    deps: ResumeActionDeps,
) -> str:
    if channel_id is None:
        message = "Discord interaction has no channel ID."
        deps.log(f"resident_thread_resume_button_failed error={message}")
        return f"Resume failed\n\nERROR: {message}\n\nNo prompt was resent."
    try:
        return await deps.recover_resident_thread_for_request(channel_id, target_thread_id)
    except (OSError, RuntimeError, ValueError) as exc:
        deps.log("resident_thread_resume_button_failed\n" + traceback.format_exc())
        return f"Resume failed\n\nERROR: {exc}\n\nNo prompt was resent."


class ResumeView(discord.ui.View):
    def __init__(self, target_thread_id: str, *, deps: ResumeViewDeps) -> None:
        super().__init__(timeout=900)
        self.target_thread_id: str = target_thread_id
        self.deps: ResumeViewDeps = deps

    @override
    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if self.deps.is_user_allowed(interaction.user.id):
            return True
        self.deps.log(
            f"resume_button_denied user={interaction.user.id} target={self.target_thread_id}"
        )
        await self.deps.send_interaction_response(
            interaction,
            "This user is not allowed.",
            ephemeral=True,
            context="resume_button_denied",
        )
        return False

    @discord.ui.button(label="Resume", style=discord.ButtonStyle.primary)
    async def resume(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        _ = await interaction.response.defer(thinking=True, ephemeral=True)
        response = await build_resume_button_response(
            interaction.channel_id,
            self.target_thread_id,
            deps=self.deps,
        )
        await self.deps.send_direct_followup(
            interaction,
            response,
            ephemeral=True,
            log_prefix="resume_button_followup",
            context="resume_button",
        )

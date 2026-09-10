from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol, cast, override

import discord

import codex_discord_approval_button_action as discord_approval_button_action
import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_component_view_state as discord_component_view_state

LogFunc = Callable[[str], None]
AllowedUserChecker = Callable[[int], bool]
ExceptionFormatter = Callable[[], str]
ApprovalActionDepsFactory = Callable[[], discord_approval_button_action.ApprovalButtonActionDeps]


class InteractionResponseSender(Protocol):
    def __call__(
        self,
        interaction: discord.Interaction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


class InteractionMessageResolver(Protocol):
    def __call__(self, interaction: discord.Interaction) -> discord.Message: ...


@dataclass(frozen=True, slots=True)
class ApprovalViewDeps:
    is_user_allowed: AllowedUserChecker
    send_interaction_response: InteractionResponseSender
    require_interaction_message: InteractionMessageResolver
    delivery_exceptions: tuple[type[BaseException], ...]
    format_exception: ExceptionFormatter
    make_action_deps: ApprovalActionDepsFactory
    log: LogFunc


class ApprovalView(discord.ui.View):
    def __init__(self, target_thread_id: str, *, deps: ApprovalViewDeps) -> None:
        super().__init__(timeout=1800)
        self.target_thread_id: str = target_thread_id
        self.deps: ApprovalViewDeps = deps
        self.claimed: bool = False
        self.assign_persistent_custom_ids()

    def assign_persistent_custom_ids(self) -> None:
        discord_component_view_state.assign_approval_button_custom_ids(
            self._component_children(),
            self.target_thread_id,
            is_button=discord_bot_shapes.is_discord_button_item,
        )

    def _component_children(self) -> Iterable[discord_component_view_state.ComponentViewChild]:
        return cast(Iterable[discord_component_view_state.ComponentViewChild], self.children)

    @override
    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if self.deps.is_user_allowed(interaction.user.id):
            return True
        self.deps.log(f"approval_button_denied user={interaction.user.id} target={self.target_thread_id}")
        await self.deps.send_interaction_response(
            interaction,
            "This user is not allowed.",
            ephemeral=True,
            context="approval_button_denied",
        )
        return False

    async def _submit(self, interaction: discord.Interaction, answer: str) -> None:
        if self.claimed:
            await self.deps.send_interaction_response(
                interaction,
                "This approval choice was already handled.",
                ephemeral=True,
                context="approval_button_already_handled",
            )
            return
        self.claimed = True
        self.disable_all_items()
        _ = await interaction.response.defer(thinking=True, ephemeral=True)
        try:
            _ = await self.deps.require_interaction_message(interaction).edit(view=self)
        except self.deps.delivery_exceptions:
            self.deps.log("approval_button_message_edit_failed\n" + self.deps.format_exception())
        await discord_approval_button_action.handle_approval_button_submit(
            interaction,
            self.target_thread_id,
            answer,
            deps=self.deps.make_action_deps(),
        )

    @discord.ui.button(label="Approve", style=discord.ButtonStyle.success)
    async def approve(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        await self._submit(interaction, "1")

    @discord.ui.button(label="Approve session", style=discord.ButtonStyle.primary)
    async def approve_session(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        await self._submit(interaction, "2")

    @discord.ui.button(label="Reject", style=discord.ButtonStyle.danger)
    async def reject(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        await self._submit(interaction, "3")

    @discord.ui.button(label="Cancel", style=discord.ButtonStyle.secondary)
    async def cancel(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        await self._submit(interaction, "cancel")

    def disable_all_items(self) -> None:
        for raw_item in self.children:
            item = cast(discord_component_view_state.ComponentViewChild, cast(object, raw_item))
            if discord_bot_shapes.is_discord_button_item(item):
                item.disabled = True

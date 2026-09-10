from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol, cast, override

import discord

import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_busy_choice_queue_action as discord_busy_choice_queue_action
import codex_discord_busy_choice_steer_action as discord_busy_choice_steer_action
import codex_discord_busy_choice_stop_action as discord_busy_choice_stop_action
import codex_discord_component_view_state as discord_component_view_state

LogFunc = Callable[[str], None]
BusyChoiceRecordClaimer = Callable[[str], bool]
SteerActionDepsFactory = Callable[[], discord_busy_choice_steer_action.BusyChoiceSteerActionDeps]
QueueActionDepsFactory = Callable[[], discord_busy_choice_queue_action.BusyChoiceQueueActionDeps]
StopActionDepsFactory = Callable[[], discord_busy_choice_stop_action.BusyChoiceStopActionDeps]


class BusyInteractionResponseSender(Protocol):
    def __call__(
        self,
        interaction: discord.Interaction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


class BusyDirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: discord.Interaction,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


class BusyComponentClearer(Protocol):
    def __call__(self, interaction: discord.Interaction, *, context: str) -> Awaitable[None]: ...


class BusyChoiceViewMessage(Protocol):
    @property
    def channel(self) -> discord_busy_choice_stop_action.StopActionChannel: ...

    @property
    def author(self) -> discord_bot_shapes.BusyChoiceAuthor: ...


@dataclass(frozen=True, slots=True)
class BusyChoiceViewDeps:
    claim_busy_choice_record: BusyChoiceRecordClaimer
    send_interaction_response: BusyInteractionResponseSender
    send_direct_followup: BusyDirectFollowupSender
    clear_interaction_message_components: BusyComponentClearer
    make_steer_action_deps: SteerActionDepsFactory
    make_queue_action_deps: QueueActionDepsFactory
    make_stop_action_deps: StopActionDepsFactory
    log: LogFunc


class BusyChoiceView(discord.ui.View):
    def __init__(
        self,
        message: BusyChoiceViewMessage,
        prompt: str,
        *,
        deps: BusyChoiceViewDeps,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
        choice_id: str | None = None,
    ) -> None:
        super().__init__(timeout=900)
        self.message: BusyChoiceViewMessage = message
        self.prompt: str = prompt
        self.deps: BusyChoiceViewDeps = deps
        self.target_thread_id: str | None = target_thread_id
        self.allow_steer: bool = allow_steer
        self.choice_id: str | None = choice_id
        self.claimed: bool = False
        self.assign_persistent_custom_ids()

    def assign_persistent_custom_ids(self) -> None:
        discord_component_view_state.configure_busy_choice_buttons(
            self._component_children(),
            self.choice_id,
            allow_steer=self.allow_steer,
            is_button=discord_bot_shapes.is_discord_button_item,
        )

    def _component_children(self) -> Iterable[discord_component_view_state.ComponentViewChild]:
        return cast(Iterable[discord_component_view_state.ComponentViewChild], self.children)

    @override
    async def interaction_check(self, interaction: discord.Interaction) -> bool:
        if interaction.user.id == self.message.author.id:
            return True
        self.deps.log(
            f"busy_choice_denied user={interaction.user.id} owner={self.message.author.id} target={self.target_thread_id or '-'}"
        )
        await self.deps.send_interaction_response(
            interaction,
            "Only the original sender can choose this.",
            ephemeral=True,
            context="busy_choice_denied",
        )
        return False

    def claim(self) -> bool:
        if self.claimed:
            return False
        if self.choice_id and not self.deps.claim_busy_choice_record(self.choice_id):
            return False
        self.claimed = True
        self.disable_all_items()
        return True

    async def _send_already_handled(
        self,
        interaction: discord.Interaction,
        *,
        action: str,
        context: str,
        followup: bool,
    ) -> None:
        self.deps.log(f"busy_choice_already_handled action={action} user={interaction.user.id} target={self.target_thread_id or '-'}")
        if not followup:
            await self.deps.send_interaction_response(
                interaction,
                "This busy choice was already handled.",
                ephemeral=True,
                context=context,
            )
            return
        await self.deps.send_direct_followup(
            interaction,
            "This busy choice was already handled.",
            log_prefix="button_followup",
            context=context,
        )

    @discord.ui.button(label="Steer now", style=discord.ButtonStyle.primary)
    async def steer_now(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        if not self.allow_steer:
            await self.deps.send_interaction_response(
                interaction,
                "This message targets a different Codex thread. Queue it instead.",
                ephemeral=True,
                context="steer_now_not_allowed",
            )
            self.deps.log(f"steer_now_rejected user={interaction.user.id} target={self.target_thread_id or '-'} reason=not_allowed")
            return
        if self.claimed:
            await self._send_already_handled(interaction, action="steer_now", context="steer_now_already_claimed", followup=False)
            return
        _ = await interaction.response.defer(thinking=True, ephemeral=True)
        if not self.claim():
            await self._send_already_handled(interaction, action="steer_now", context="steer_now_already_handled", followup=True)
            return
        await self.deps.clear_interaction_message_components(interaction, context="busy_choice_steer")
        await discord_busy_choice_steer_action.handle_busy_choice_steer_action(
            interaction,
            self.message.channel,
            self.prompt,
            self.target_thread_id,
            user_id=interaction.user.id,
            deps=self.deps.make_steer_action_deps(),
        )

    @discord.ui.button(label="Queue next", style=discord.ButtonStyle.secondary)
    async def queue_next(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        if self.claimed:
            await self._send_already_handled(interaction, action="queue_next", context="queue_next_already_claimed", followup=False)
            return
        _ = await interaction.response.defer(thinking=False)
        if not self.claim():
            await self._send_already_handled(interaction, action="queue_next", context="queue_next_already_handled", followup=True)
            return
        await self.deps.clear_interaction_message_components(interaction, context="busy_choice_queue")
        await discord_busy_choice_queue_action.handle_busy_choice_queue_action(
            interaction,
            self.message.channel,
            self.message,
            prompt=self.prompt,
            target_thread_id=self.target_thread_id,
            user_id=interaction.user.id,
            deps=self.deps.make_queue_action_deps(),
        )

    @discord.ui.button(label="Stop reply", style=discord.ButtonStyle.danger)
    async def stop_reply(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        if self.claimed:
            await self._send_already_handled(interaction, action="stop_reply", context="stop_reply_already_claimed", followup=False)
            return
        _ = await interaction.response.defer(thinking=True, ephemeral=True)
        if not self.claim():
            await self._send_already_handled(interaction, action="stop_reply", context="stop_reply_already_handled", followup=True)
            return
        await self.deps.clear_interaction_message_components(interaction, context="busy_choice_stop")
        await discord_busy_choice_stop_action.handle_busy_choice_stop_action(
            interaction,
            self.message.channel,
            self.target_thread_id,
            user_id=interaction.user.id,
            deps=self.deps.make_stop_action_deps(),
        )

    @discord.ui.button(label="Ignore", style=discord.ButtonStyle.secondary)
    async def ignore(self, interaction: discord.Interaction, button: discord.ui.Button[discord.ui.View]) -> None:
        _ = button
        if self.claimed:
            await self._send_already_handled(interaction, action="ignore", context="ignore_already_claimed", followup=False)
            return
        _ = await interaction.response.defer(thinking=True)
        if not self.claim():
            await self._send_already_handled(interaction, action="ignore", context="ignore_already_handled", followup=True)
            return
        self.deps.log(f"ignore_busy_prompt user={interaction.user.id} target={self.target_thread_id or '-'}")
        await self.deps.clear_interaction_message_components(interaction, context="busy_choice_ignore")
        await self.deps.send_direct_followup(
            interaction,
            "Ignored.",
            log_prefix="button_followup",
            context="ignore",
        )
        self.deps.log(f"ignore_busy_prompt_sent user={interaction.user.id} target={self.target_thread_id or '-'}")

    def disable_all_items(self) -> None:
        for raw_item in self.children:
            item = cast(discord_component_view_state.ComponentViewChild, cast(object, raw_item))
            if discord_bot_shapes.is_discord_button_item(item):
                item.disabled = True

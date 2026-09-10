from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Protocol, cast, override

import discord

import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_component_view_state as discord_component_view_state
import codex_discord_input_choice_button_action as discord_input_choice_button_action
from codex_discord_components import format_input_choice_custom_id

LogFunc = Callable[[str], None]
AllowedUserChecker = Callable[[int], bool]
ExceptionFormatter = Callable[[], str]
InputActionDepsFactory = Callable[[], discord_input_choice_button_action.InputChoiceButtonActionDeps]


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
class InputChoiceViewDeps:
    is_user_allowed: AllowedUserChecker
    send_interaction_response: InteractionResponseSender
    require_interaction_message: InteractionMessageResolver
    delivery_exceptions: tuple[type[BaseException], ...]
    format_exception: ExceptionFormatter
    make_action_deps: InputActionDepsFactory
    log: LogFunc


class InputChoiceButton(discord.ui.Button[discord.ui.View]):
    def __init__(
        self,
        target_thread_id: str,
        value: str,
        label: str,
        *,
        deps: InputChoiceViewDeps,
    ) -> None:
        super().__init__(
            label=label[:80],
            style=discord.ButtonStyle.primary,
            custom_id=format_input_choice_custom_id(target_thread_id, value),
        )
        self.target_thread_id: str = target_thread_id
        self.value: str = value
        self.deps: InputChoiceViewDeps = deps

    @override
    async def callback(self, interaction: discord.Interaction) -> None:
        if not self.deps.is_user_allowed(interaction.user.id):
            self.deps.log(f"input_choice_button_denied user={interaction.user.id} target={self.target_thread_id}")
            await self.deps.send_interaction_response(
                interaction,
                "This user is not allowed.",
                ephemeral=True,
                context="input_choice_button_denied",
            )
            return
        view = self.view
        if isinstance(view, InputChoiceView) and not view.claim():
            await self.deps.send_interaction_response(
                interaction,
                "This input choice was already handled.",
                ephemeral=True,
                context="input_choice_button_already_handled",
            )
            return
        _ = await interaction.response.defer(thinking=True, ephemeral=True)
        if isinstance(view, InputChoiceView):
            try:
                _ = await self.deps.require_interaction_message(interaction).edit(view=view)
            except self.deps.delivery_exceptions:
                self.deps.log("input_choice_button_message_edit_failed\n" + self.deps.format_exception())
        await discord_input_choice_button_action.handle_input_choice_button_submit(
            interaction,
            self.target_thread_id,
            self.value,
            deps=self.deps.make_action_deps(),
        )


class InputChoiceView(discord.ui.View):
    def __init__(
        self,
        target_thread_id: str,
        options: Sequence[tuple[str, str]],
        *,
        deps: InputChoiceViewDeps,
    ) -> None:
        super().__init__(timeout=1800)
        self.claimed: bool = False
        self.deps: InputChoiceViewDeps = deps
        for value, label in options[:5]:
            _ = self.add_item(InputChoiceButton(target_thread_id, value, label, deps=deps))

    def claim(self) -> bool:
        if self.claimed:
            return False
        self.claimed = True
        for raw_item in self.children:
            item = cast(discord_component_view_state.ComponentViewChild, cast(object, raw_item))
            if discord_bot_shapes.is_discord_button_item(item):
                item.disabled = True
        return True

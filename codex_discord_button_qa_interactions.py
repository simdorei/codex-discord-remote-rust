from __future__ import annotations

from types import SimpleNamespace
from typing import TypeAlias, cast

import discord

import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_button_qa_lifecycle_cases as discord_button_qa_lifecycle_cases
import codex_discord_button_qa_persistent_cases as discord_button_qa_persistent_cases
import codex_discord_button_qa_steer_case as discord_button_qa_steer_case

SyntheticQAFollowupMessage: TypeAlias = str | tuple[str, discord_bot_shapes.SyntheticQAView]


class SyntheticQAResponse:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.deferred: bool = False
        self.done: bool = False
        self.defer_kwargs: list[dict[str, bool]] = []

    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        _ = ephemeral
        self.messages.append(content)
        self.done = True

    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        self.deferred = True
        self.done = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})

    def is_done(self) -> bool:
        return self.done


class SyntheticQAFollowup:
    def __init__(self) -> None:
        self.messages: list[SyntheticQAFollowupMessage] = []
        self.kwargs: list[dict[str, bool]] = []

    async def send(
        self,
        content: str,
        view: discord_bot_shapes.SyntheticQAView | None = None,
        **kwargs: bool,
    ) -> None:
        self.messages.append(content if view is None else (content, view))
        self.kwargs.append(kwargs)


class SyntheticQAInteraction:
    def __init__(
        self,
        *,
        bot: discord_bot_shapes.SyntheticQABot,
        channel: discord_bot_shapes.SyntheticQAChannel,
        message: discord_bot_shapes.SyntheticQAMessage,
        user: discord_bot_shapes.SyntheticQAUser,
        custom_id: str,
    ) -> None:
        channel_id = getattr(channel, "id", None)
        self.client: discord_bot_shapes.SyntheticQABot = bot
        self.channel: discord_bot_shapes.SyntheticQAChannel = channel
        self.channel_id: int | None = channel_id if isinstance(channel_id, int) else None
        self.command: SimpleNamespace = SimpleNamespace(name="-")
        self.data: dict[str, str] = {"custom_id": custom_id}
        self.followup: SyntheticQAFollowup = SyntheticQAFollowup()
        self.message: discord_bot_shapes.SyntheticQAMessage = message
        self.response: SyntheticQAResponse = SyntheticQAResponse()
        self.type: discord.InteractionType = discord.InteractionType.component
        self.user: discord_bot_shapes.SyntheticQAUser = user


def make_lifecycle_qa_interaction(
    *,
    bot: discord_button_qa_lifecycle_cases.LifecycleQaBot,
    channel: discord_button_qa_lifecycle_cases.LifecycleQaChannel,
    message: discord_button_qa_lifecycle_cases.LifecycleQaMessage,
    user: discord_button_qa_lifecycle_cases.LifecycleQaUser,
    custom_id: str,
) -> discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction:
    interaction = SyntheticQAInteraction(
        bot=bot,
        channel=channel,
        message=message,
        user=user,
        custom_id=custom_id,
    )
    return cast(discord_button_qa_lifecycle_cases.BusyChoiceQaInteraction, interaction)


def make_steer_qa_interaction(
    *,
    bot: discord_button_qa_steer_case.SteerQaBot,
    channel: discord_button_qa_steer_case.SteerQaChannel,
    message: discord_button_qa_steer_case.SteerQaMessage,
    user: discord_button_qa_steer_case.SteerQaUser,
    custom_id: str,
) -> discord_button_qa_steer_case.SteerQaInteraction:
    interaction = SyntheticQAInteraction(
        bot=bot,
        channel=channel,
        message=message,
        user=user,
        custom_id=custom_id,
    )
    return cast(discord_button_qa_steer_case.SteerQaInteraction, cast(object, interaction))


def make_persistent_qa_interaction(
    *,
    bot: discord_button_qa_persistent_cases.PersistentQaBot,
    channel: discord_button_qa_persistent_cases.PersistentQaChannel,
    message: discord_button_qa_persistent_cases.PersistentQaMessage,
    user: discord_button_qa_persistent_cases.PersistentQaUser,
    custom_id: str,
) -> discord_button_qa_persistent_cases.PersistentQaInteraction:
    interaction = SyntheticQAInteraction(
        bot=bot,
        channel=channel,
        message=message,
        user=user,
        custom_id=custom_id,
    )
    return cast(discord_button_qa_persistent_cases.PersistentQaInteraction, cast(object, interaction))

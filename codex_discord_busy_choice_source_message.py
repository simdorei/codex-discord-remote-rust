from __future__ import annotations

from typing import Protocol, TypeAlias, cast

import discord

import codex_discord_bot_shapes as discord_bot_shapes

BusyChoiceAuthorCandidate: TypeAlias = discord_bot_shapes.BusyChoiceAuthor | None
BusyChoiceChannelCandidate: TypeAlias = discord.abc.Messageable | None


class BusyChoiceChannelTypeError(TypeError):
    def __init__(self, channel: BusyChoiceChannelCandidate) -> None:
        super().__init__(f"Expected messageable button QA channel, got {type(channel).__name__}")


class BusyChoiceAuthorTypeError(TypeError):
    def __init__(self, author: BusyChoiceAuthorCandidate) -> None:
        super().__init__(f"Expected button QA author with int id, got {type(author).__name__}")


class RuntimeBusyChoiceMessageLike(Protocol):
    @property
    def author(self) -> BusyChoiceAuthorCandidate: ...

    @property
    def channel(self) -> BusyChoiceChannelCandidate: ...


def make_busy_choice_source_message(
    author: BusyChoiceAuthorCandidate,
    channel: BusyChoiceChannelCandidate,
) -> discord_bot_shapes.RuntimeBusyChoiceSourceMessage:
    if not isinstance(channel, discord.abc.Messageable):
        raise BusyChoiceChannelTypeError(channel)
    author_id = getattr(author, "id", None)
    if not isinstance(author_id, int):
        raise BusyChoiceAuthorTypeError(author)
    author_bot = getattr(author, "bot", False)
    return discord_bot_shapes.RuntimeBusyChoiceSourceMessage(
        author=discord_bot_shapes.RuntimeBusyChoiceAuthor(
            id=author_id,
            bot=bool(author_bot),
        ),
        channel=cast(discord_bot_shapes.DiscordMessageableChannel, channel),
    )


def make_runtime_busy_choice_source_message(
    message: RuntimeBusyChoiceMessageLike,
) -> discord_bot_shapes.RuntimeBusyChoiceSourceMessage:
    return make_busy_choice_source_message(message.author, message.channel)

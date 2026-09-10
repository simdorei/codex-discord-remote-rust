from __future__ import annotations

import os
from typing import Protocol

from codex_discord_text import parse_int_set


class LogFunc(Protocol):
    def __call__(self, message: str, /) -> None: ...


DiscordIdValue = int | str | bytes | bytearray | None


class InteractionUserLike(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class InteractionChannelLike(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class InteractionLike(Protocol):
    @property
    def user(self) -> InteractionUserLike | None: ...

    @property
    def channel_id(self) -> DiscordIdValue: ...

    @property
    def channel(self) -> InteractionChannelLike | None: ...


class CommandNameFunc(Protocol):
    def __call__(self, interaction: InteractionLike, /) -> str: ...


class MirroredChannelFunc(Protocol):
    def __call__(self, channel_id: DiscordIdValue, /) -> bool: ...


class InteractionGateBot(Protocol):
    def is_allowed_user(self, user_id: DiscordIdValue) -> bool: ...

    def is_allowed_channel(self, channel_id: DiscordIdValue) -> bool: ...

    def is_allowed_message_channel(self, channel: InteractionChannelLike) -> bool: ...


def is_discord_user_allowed(user_id: int | None) -> bool:
    allowed_user_ids = parse_int_set(os.environ.get("DISCORD_ALLOWED_USER_IDS", ""))
    if not allowed_user_ids:
        return True
    return user_id in allowed_user_ids


def check_interaction_allowed(
    bot: InteractionGateBot,
    interaction: InteractionLike,
    *,
    log_func: LogFunc,
    get_interaction_command_name_func: CommandNameFunc,
    is_mirrored_channel_id_func: MirroredChannelFunc,
) -> bool:
    command_name = get_interaction_command_name_func(interaction)
    user = interaction.user
    user_id = None if user is None else user.id
    channel_id = interaction.channel_id
    if not bot.is_allowed_user(user_id):
        log_func(
            f"slash_ignored command={command_name} reason=user_not_allowed "
            + f"user={user_id} channel={channel_id}"
        )
        return False
    if bot.is_allowed_channel(channel_id):
        return True
    if is_mirrored_channel_id_func(channel_id):
        return True
    channel = interaction.channel
    if channel is not None and bot.is_allowed_message_channel(channel):
        return True
    log_func(
        f"slash_ignored command={command_name} reason=channel_not_allowed "
        + f"user={user_id} channel={channel_id}"
    )
    return False

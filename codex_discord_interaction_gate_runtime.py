from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, TypeAlias, cast

import discord

import codex_discord_id_values as discord_id_values
import codex_discord_interaction_gate as discord_interaction_gate

DiscordMessageChannel: TypeAlias = (
    discord.abc.GuildChannel | discord.Thread | discord.abc.PrivateChannel
)


class RuntimeInteractionGateBot(Protocol):
    def is_allowed_user(self, user_id: int | None) -> bool: ...

    def is_allowed_channel(self, channel_id: int | None) -> bool: ...

    def is_allowed_message_channel(self, channel: DiscordMessageChannel) -> bool: ...


@dataclass(frozen=True, slots=True)
class InteractionGateBotAdapter:
    bot: RuntimeInteractionGateBot

    def is_allowed_user(self, user_id: discord_interaction_gate.DiscordIdValue) -> bool:
        return self.bot.is_allowed_user(discord_id_values.coerce_discord_id_value(user_id))

    def is_allowed_channel(self, channel_id: discord_interaction_gate.DiscordIdValue) -> bool:
        return self.bot.is_allowed_channel(discord_id_values.coerce_discord_id_value(channel_id))

    def is_allowed_message_channel(
        self,
        channel: discord_interaction_gate.InteractionChannelLike,
    ) -> bool:
        channel_obj = cast(object, channel)
        if isinstance(channel_obj, DiscordMessageChannel):
            return self.bot.is_allowed_message_channel(channel_obj)
        return False

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, cast

import codex_discord_delivery_state as discord_delivery_state
import codex_discord_id_values as discord_id_values
import codex_discord_interaction_gate as discord_interaction_gate


class InteractionCommandLike(Protocol):
    @property
    def name(self) -> str: ...


class InteractionCommandSource(Protocol):
    @property
    def command(self) -> InteractionCommandLike | None: ...


@dataclass(frozen=True, slots=True)
class InteractionChannelRuntime:
    is_mirrored_channel_id_func: Callable[[int | None], bool]

    def get_interaction_gate_command_name(self, interaction: discord_interaction_gate.InteractionLike) -> str:
        try:
            command = cast(InteractionCommandSource, cast(object, interaction)).command
        except AttributeError:
            return "-"
        return "-" if command is None else str(command.name or "-")

    def coerce_interaction_channel_id(
        self,
        channel_id: discord_interaction_gate.DiscordIdValue,
    ) -> int | None:
        return discord_id_values.coerce_discord_id_value(channel_id)

    def coerce_delivery_state_discord_id(
        self,
        discord_id: discord_delivery_state.DiscordIdValue,
    ) -> int | None:
        return discord_id_values.coerce_discord_id_value(discord_id)

    def is_mirrored_interaction_channel_id(
        self,
        channel_id: discord_interaction_gate.DiscordIdValue,
    ) -> bool:
        return self.is_mirrored_channel_id_func(self.coerce_interaction_channel_id(channel_id))

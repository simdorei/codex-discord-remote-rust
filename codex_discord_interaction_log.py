from __future__ import annotations

from collections.abc import Mapping
from typing import Protocol, TypeAlias

from codex_discord_text import format_discord_command_label

InteractionScalar: TypeAlias = str | int | bool | None
InteractionDataValue: TypeAlias = InteractionScalar | Mapping[str, InteractionScalar]
RawInteractionData: TypeAlias = Mapping[str, InteractionDataValue]


class NamedInteractionType(Protocol):
    @property
    def name(self) -> str: ...


InteractionTypeValue: TypeAlias = NamedInteractionType | str | int | bool | None


UserIdValue: TypeAlias = str | int | None


class InteractionTypeLike(Protocol):
    @property
    def type(self) -> InteractionTypeValue: ...


class DiscordUserIdLike(Protocol):
    @property
    def id(self) -> UserIdValue: ...


DiscordUserLogValue: TypeAlias = DiscordUserIdLike | str | int | None


class InteractionDataLike(Protocol):
    @property
    def data(self) -> RawInteractionData | None: ...


def format_interaction_type(interaction: InteractionTypeLike) -> str:
    interaction_type = interaction.type
    name = _interaction_type_name(interaction_type)
    return str(name or interaction_type or "-")


def format_raw_interaction_command(data: RawInteractionData) -> str:
    interaction_data = data.get("data")
    if not isinstance(interaction_data, Mapping):
        return "-"
    name = interaction_data.get("name")
    if name:
        return format_discord_command_label(str(name), limit=80)
    custom_id = interaction_data.get("custom_id")
    if custom_id:
        return format_discord_command_label(str(custom_id), limit=100)
    return "-"


def get_interaction_custom_id(interaction: InteractionDataLike) -> str:
    data = interaction.data
    if not isinstance(data, Mapping):
        return "-"
    custom_id = data.get("custom_id")
    if custom_id is None:
        return "-"
    return format_discord_command_label(str(custom_id), limit=100)


def format_discord_user_id_for_log(user: DiscordUserLogValue) -> str:
    return str(getattr(user, "id", None) or "-")


def _interaction_type_name(value: InteractionTypeValue) -> str | None:
    if value is None or isinstance(value, str | int | bool):
        return None
    return value.name

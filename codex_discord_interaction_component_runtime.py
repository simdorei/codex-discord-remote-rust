from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_delivery_state import DiscordIdValue, Messageable

LogFunc = Callable[[str], None]
FormatExceptionFunc = Callable[[], str]


class ComponentInteractionUser(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...


class EditableComponentMessage(Protocol):
    async def edit(self, *, view: None = None) -> None: ...


class ComponentInteraction(Protocol):
    @property
    def message(self) -> EditableComponentMessage | None: ...

    @property
    def channel_id(self) -> DiscordIdValue: ...

    @property
    def user(self) -> ComponentInteractionUser: ...


class InteractionChannelClient(Protocol):
    async def fetch_channel(self, channel_id: int) -> Messageable | None: ...


class InteractionChannelSource(Protocol):
    @property
    def channel(self) -> Messageable | None: ...

    @property
    def client(self) -> InteractionChannelClient | None: ...


@dataclass(frozen=True, slots=True)
class InteractionComponentRuntime:
    delivery_exceptions: tuple[type[BaseException], ...]
    format_exception: FormatExceptionFunc
    log: LogFunc

    async def clear_interaction_message_components(
        self,
        interaction: ComponentInteraction,
        *,
        context: str,
    ) -> None:
        message = interaction.message
        if message is None:
            return
        try:
            await message.edit(view=None)
            self.log(
                f"component_message_components_cleared context={context} "
                + f"channel={interaction.channel_id} user={interaction.user.id or '-'}"
            )
        except self.delivery_exceptions:
            self.log(
                f"component_message_components_clear_failed context={context}\n"
                + self.format_exception()
            )

    async def resolve_interaction_channel(
        self,
        interaction: InteractionChannelSource,
        channel_id: int,
    ) -> Messageable | None:
        channel = interaction.channel
        if channel is not None:
            return channel
        client = interaction.client
        if client is not None:
            try:
                return await client.fetch_channel(channel_id)
            except self.delivery_exceptions as exc:
                self.log(
                    f"busy_choice_persistent_channel_fetch_failed channel={channel_id} "
                    + f"error_type={type(exc).__name__}"
                )
        return None

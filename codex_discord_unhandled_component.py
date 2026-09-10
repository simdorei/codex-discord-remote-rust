from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


STALE_COMPONENT_NOTICE = "This Discord button is no longer active. Send the message again to get fresh controls."

InteractionT = TypeVar("InteractionT", bound="UnhandledInteraction")
InteractionContraT = TypeVar("InteractionContraT", bound="UnhandledInteraction", contravariant=True)
ExceptionTypes = tuple[type[BaseException], ...]


class UnhandledResponse(Protocol):
    def is_done(self) -> bool: ...


class UnhandledUser(Protocol):
    @property
    def id(self) -> int: ...


class UnhandledInteraction(Protocol):
    @property
    def response(self) -> UnhandledResponse: ...

    @property
    def channel_id(self) -> int | None: ...

    @property
    def user(self) -> UnhandledUser: ...


class PersistentHandler(Protocol[InteractionContraT]):
    def __call__(self, interaction: InteractionContraT, custom_id: str) -> Awaitable[bool]: ...


class ComponentClearer(Protocol[InteractionContraT]):
    def __call__(self, interaction: InteractionContraT, *, context: str) -> Awaitable[None]: ...


class InteractionResponder(Protocol[InteractionContraT]):
    def __call__(
        self,
        interaction: InteractionContraT,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class UnhandledComponentDeps(Generic[InteractionT]):
    sleep: Callable[[float], Awaitable[None]]
    get_custom_id: Callable[[InteractionT], str]
    persistent_handlers: Sequence[PersistentHandler[InteractionT]]
    clear_components: ComponentClearer[InteractionT]
    send_response: InteractionResponder[InteractionT]
    is_already_acknowledged: Callable[[BaseException], bool]
    format_exception: Callable[[], str]
    delivery_exceptions: ExceptionTypes
    log: Callable[[str], None]


async def report_unhandled_component_interaction(
    interaction: InteractionT,
    *,
    delay_sec: float = 0.75,
    deps: UnhandledComponentDeps[InteractionT],
) -> None:
    await deps.sleep(delay_sec)
    if interaction.response.is_done():
        return
    custom_id = deps.get_custom_id(interaction)
    try:
        for handler in deps.persistent_handlers:
            if await handler(interaction, custom_id):
                return
    except deps.delivery_exceptions as exc:
        if deps.is_already_acknowledged(exc):
            _log_already_acknowledged(
                "component_interaction_persistent_handler_already_acknowledged",
                interaction,
                custom_id,
                deps,
            )
            return
        deps.log("component_interaction_persistent_handler_failed\n" + deps.format_exception())
        if interaction.response.is_done():
            return
    try:
        await deps.clear_components(interaction, context="unhandled_component")
        await deps.send_response(
            interaction,
            STALE_COMPONENT_NOTICE,
            ephemeral=True,
            context="component_unhandled",
        )
        deps.log(
            f"component_interaction_unhandled_reported custom_id={custom_id} "
            f"channel={interaction.channel_id} user={interaction.user.id}"
        )
    except deps.delivery_exceptions as exc:
        if deps.is_already_acknowledged(exc):
            _log_already_acknowledged(
                "component_interaction_unhandled_report_already_acknowledged",
                interaction,
                custom_id,
                deps,
            )
            return
        deps.log("component_interaction_unhandled_report_failed\n" + deps.format_exception())


def _log_already_acknowledged(
    event: str,
    interaction: InteractionT,
    custom_id: str,
    deps: UnhandledComponentDeps[InteractionT],
) -> None:
    deps.log(
        f"{event} custom_id={custom_id} "
        f"channel={interaction.channel_id} user={interaction.user.id}"
    )

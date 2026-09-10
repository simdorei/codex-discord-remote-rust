from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

InteractionT = TypeVar("InteractionT", bound="SlashErrorInteraction")
InteractionContraT = TypeVar("InteractionContraT", bound="SlashErrorInteraction", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultT_co = TypeVar("SendResultT_co", covariant=True)


class SlashErrorResponse(Protocol):
    def is_done(self) -> bool: ...


class SlashErrorUser(Protocol):
    @property
    def id(self) -> int: ...


class SlashErrorInteraction(Protocol):
    @property
    def response(self) -> SlashErrorResponse: ...

    @property
    def channel_id(self) -> int | None: ...

    @property
    def user(self) -> SlashErrorUser: ...


class FollowupSender(Protocol[InteractionContraT, SendResultT_co]):
    def __call__(
        self,
        interaction: InteractionContraT,
        content: str,
        *,
        ephemeral: bool,
        log_prefix: str,
        context: str,
        allow_during_stop: bool = False,
    ) -> Awaitable[SendResultT_co]: ...


class InitialResponseSender(Protocol[InteractionContraT]):
    def __call__(
        self,
        interaction: InteractionContraT,
        content: str,
        *,
        ephemeral: bool,
        context: str,
        allow_during_stop: bool = False,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class SlashCommandErrorDeps(Generic[InteractionT, SendResultT]):
    get_command_name: Callable[[InteractionT], str]
    delivery_rejected_type: type[BaseException]
    restarting_notice: str
    send_followup: FollowupSender[InteractionT, SendResultT]
    send_initial_response: InitialResponseSender[InteractionT]
    delivery_exceptions: tuple[type[BaseException], ...]
    format_exception: Callable[[], str]
    log: Callable[[str], None]


async def handle_slash_command_error(
    interaction: InteractionT,
    error: BaseException,
    *,
    deps: SlashCommandErrorDeps[InteractionT, SendResultT],
) -> None:
    command_name = deps.get_command_name(interaction)
    deps.log(
        f"slash_command_error command={command_name} "
        + f"channel={interaction.channel_id} user={getattr(interaction.user, 'id', '-')} "
        + f"error={type(error).__name__}: {error}"
    )
    try:
        root_error = getattr(error, "original", error)
        restarting = isinstance(root_error, deps.delivery_rejected_type)
        message = (
            deps.restarting_notice
            if restarting
            else "Discord slash command error. Check codex_discord_bot.log."
        )
        if interaction.response.is_done():
            _ = await deps.send_followup(
                interaction,
                message,
                ephemeral=True,
                log_prefix="slash_command_error",
                context="error_followup",
                allow_during_stop=restarting,
            )
            deps.log(f"slash_command_error_sent command={command_name} response=followup")
        else:
            await deps.send_initial_response(
                interaction,
                message,
                ephemeral=True,
                context="slash_command_error",
                allow_during_stop=restarting,
            )
            deps.log(f"slash_command_error_sent command={command_name} response=initial")
    except deps.delivery_exceptions:
        deps.log("slash_command_error_report_failed\n" + deps.format_exception())

from __future__ import annotations

from collections.abc import Callable
from typing import Protocol


class DiscordErrorsLike(Protocol):
    InteractionResponded: type[BaseException] | None


def is_interaction_already_acknowledged_error(
    exc: BaseException,
    *,
    interaction_responded_type: type[BaseException] | None,
) -> bool:
    if getattr(exc, "code", None) == 40060:
        return True
    if interaction_responded_type is not None and isinstance(exc, interaction_responded_type):
        return True
    message = str(exc).lower()
    return "already been acknowledged" in message or "already been responded" in message


def get_interaction_responded_type(
    discord_errors: DiscordErrorsLike,
) -> type[BaseException] | None:
    return discord_errors.InteractionResponded


def make_interaction_already_acknowledged_error_checker(
    discord_errors: DiscordErrorsLike,
) -> Callable[[BaseException], bool]:
    def is_already_acknowledged(exc: BaseException) -> bool:
        return is_interaction_already_acknowledged_error(
            exc,
            interaction_responded_type=get_interaction_responded_type(discord_errors),
        )

    return is_already_acknowledged

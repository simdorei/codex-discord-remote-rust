from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable, Sequence
import traceback
from typing import TypeAlias, cast

import codex_discord_interaction_log as discord_interaction_log
import codex_discord_unhandled_component as discord_unhandled_component

UnhandledComponentInteraction: TypeAlias = discord_unhandled_component.UnhandledInteraction
UnhandledComponentHandler: TypeAlias = discord_unhandled_component.PersistentHandler[
    UnhandledComponentInteraction
]
UnhandledComponentClearer: TypeAlias = discord_unhandled_component.ComponentClearer[
    UnhandledComponentInteraction
]
UnhandledComponentResponder: TypeAlias = discord_unhandled_component.InteractionResponder[
    UnhandledComponentInteraction
]
DeliveryExceptions: TypeAlias = tuple[type[BaseException], ...]


def _get_custom_id(value: UnhandledComponentInteraction) -> str:
    return discord_interaction_log.get_interaction_custom_id(
        cast(discord_interaction_log.InteractionDataLike, cast(object, value))
    )


async def report_discord_unhandled_component_interaction(
    interaction: UnhandledComponentInteraction,
    *,
    delay_sec: float = 0.75,
    persistent_handlers: Sequence[UnhandledComponentHandler],
    clear_components: UnhandledComponentClearer,
    send_response: UnhandledComponentResponder,
    is_already_acknowledged: Callable[[BaseException], bool],
    delivery_exceptions: DeliveryExceptions,
    log: Callable[[str], None],
) -> None:
    await discord_unhandled_component.report_unhandled_component_interaction(
        interaction,
        delay_sec=delay_sec,
        deps=discord_unhandled_component.UnhandledComponentDeps(
            sleep=asyncio.sleep,
            get_custom_id=_get_custom_id,
            persistent_handlers=persistent_handlers,
            clear_components=clear_components,
            send_response=send_response,
            is_already_acknowledged=is_already_acknowledged,
            format_exception=traceback.format_exc,
            delivery_exceptions=delivery_exceptions,
            log=log,
        ),
    )


__all__ = [
    "DeliveryExceptions",
    "UnhandledComponentClearer",
    "UnhandledComponentHandler",
    "UnhandledComponentInteraction",
    "UnhandledComponentResponder",
    "report_discord_unhandled_component_interaction",
]

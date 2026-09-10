from __future__ import annotations

import traceback
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from typing import Protocol, runtime_checkable, TypeAlias

from codex_discord_components import (
    ComponentMessageLike,
    get_busy_choice_custom_ids_from_message,
    parse_busy_choice_custom_id,
)

ModuleValue: TypeAlias = object

BusyChoiceRecordValue = str | int | bool | float | None
BusyChoiceRecord = Mapping[str, BusyChoiceRecordValue]
BusyChoiceRecordGetter = Callable[[str], BusyChoiceRecord | None]
LogFunc = Callable[[str], None]


@runtime_checkable
class EditableMessage(Protocol):
    def edit(self, *, view: None = None) -> Awaitable[None]: ...


@runtime_checkable
class MessageHistoryChannel(Protocol):
    def history(self, *, limit: int) -> AsyncIterator[ComponentMessageLike]: ...


def has_active_busy_choice_custom_id(
    custom_id: str,
    *,
    get_busy_choice_record: BusyChoiceRecordGetter,
) -> bool:
    parsed = parse_busy_choice_custom_id(custom_id)
    if not parsed:
        return False
    choice_id, _action = parsed
    return get_busy_choice_record(choice_id) is not None


async def clear_stale_busy_choice_message_components(
    message: ComponentMessageLike,
    *,
    get_busy_choice_record: BusyChoiceRecordGetter,
    log_func: LogFunc,
) -> bool:
    custom_ids = get_busy_choice_custom_ids_from_message(message)
    if not custom_ids:
        return False
    if any(
        has_active_busy_choice_custom_id(custom_id, get_busy_choice_record=get_busy_choice_record)
        for custom_id in custom_ids
    ):
        return False
    if not isinstance(message, EditableMessage):
        return False
    try:
        _ = await message.edit(view=None)
        message_id = getattr(message, "id", "-")
        channel_id = getattr(getattr(message, "channel", None), "id", "-")
        log_func(f"stale_busy_choice_components_cleared message={message_id} channel={channel_id}")
        return True
    except Exception:  # noqa: BROAD_EXCEPT_OK
        log_func("stale_busy_choice_components_clear_failed\n" + traceback.format_exc())
        return False


async def cleanup_stale_busy_choice_components_in_channel(
    channel: ModuleValue | None,
    *,
    get_busy_choice_record: BusyChoiceRecordGetter,
    log_func: LogFunc,
    limit: int,
) -> int:
    if not isinstance(channel, MessageHistoryChannel):
        return 0
    cleared = 0
    async for message in channel.history(limit=limit):
        if not getattr(getattr(message, "author", None), "bot", False):
            continue
        if await clear_stale_busy_choice_message_components(
            message,
            get_busy_choice_record=get_busy_choice_record,
            log_func=log_func,
        ):
            cleared += 1
    return cleared

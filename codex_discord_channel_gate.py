from __future__ import annotations

from collections.abc import Callable
from typing import Protocol


class CategoryLike(Protocol):
    @property
    def name(self) -> str | None: ...


class ParentLike(Protocol):
    @property
    def category(self) -> CategoryLike | None: ...


class MessageChannelLike(Protocol):
    @property
    def id(self) -> int | None: ...

    @property
    def parent(self) -> ParentLike | None: ...

    @property
    def category(self) -> CategoryLike | None: ...


ChannelIdPredicate = Callable[[int | None], bool]


def is_allowed_message_channel(
    channel: MessageChannelLike,
    *,
    is_allowed_channel_func: ChannelIdPredicate,
    is_mirrored_channel_id_func: ChannelIdPredicate,
) -> bool:
    channel_id = getattr(channel, "id", None)
    parent_id = getattr(channel, "parent_id", None)
    if is_allowed_channel_func(channel_id) or is_allowed_channel_func(parent_id):
        return True
    if is_mirrored_channel_id_func(channel_id) or is_mirrored_channel_id_func(parent_id):
        return True
    category = getattr(channel, "category", None)
    parent = getattr(channel, "parent", None)
    if category is None and parent is not None:
        category = parent.category
    return category is not None and category.name == "Codex"

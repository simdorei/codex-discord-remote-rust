from __future__ import annotations

from collections.abc import Awaitable
from typing import Protocol, TypeAlias

import codex_discord_bot_shapes as discord_bot_shapes

ModuleValue: TypeAlias = object
MessageableChannel: TypeAlias = object
BusyChoiceMessage: TypeAlias = discord_bot_shapes.BusyChoiceSourceMessage
BusyChoiceViewValue: TypeAlias = object
SendResult: TypeAlias = object


class RuntimeInteractivePromptSender(Protocol):
    def __call__(
        self,
        channel: MessageableChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[tuple[str, str]],
    ) -> Awaitable[None]: ...

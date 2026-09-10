from __future__ import annotations

from collections.abc import Awaitable
from contextlib import AbstractAsyncContextManager
from typing import Protocol, TypeAlias

import codex_discord_prompt_delivery_flow as discord_prompt_delivery_flow

PromptChannel: TypeAlias = object
PromptRelay: TypeAlias = discord_prompt_delivery_flow.PromptDeliveryRelay
SendResult: TypeAlias = object
ModuleValue: TypeAlias = object


class RuntimeConfigModule(Protocol):
    def get_ask_busy_retry_attempts(self, *, default: float) -> int: ...


class DiscordAskRelayMaker(Protocol):
    def __call__(
        self,
        channel: PromptChannel,
        *,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> PromptRelay: ...


class RunAskStreamInThread(Protocol):
    def __call__(
        self,
        prompt: str,
        relay: PromptRelay,
        *,
        target_thread_id: str | None,
    ) -> Awaitable[tuple[int, str]]: ...


class PromptDeliveryChannelTyping(Protocol):
    def __call__(
        self,
        channel: PromptChannel,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]: ...

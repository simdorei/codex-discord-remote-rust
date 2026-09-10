from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Protocol

import codex_discord_stream_relay as discord_stream_relay


class PromptRelayChannel(Protocol):
    pass


class PromptChunkSender(Protocol):
    def __call__(
        self,
        channel: PromptRelayChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> Awaitable[int | None]: ...


class PromptInteractiveSender(Protocol):
    def __call__(
        self,
        channel: PromptRelayChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: list[tuple[str, str]],
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class PromptRelayChannelDeps:
    send_chunks: PromptChunkSender
    send_interactive_prompt: PromptInteractiveSender
    format_log_text_len: Callable[[str | None], int | str]


class DiscordMessageableChannelTypeError(TypeError):
    def __init__(self, channel: PromptRelayChannel) -> None:
        super().__init__(f"Expected Discord messageable channel, got {type(channel).__name__}")


@dataclass(frozen=True, slots=True)
class PromptRelayChannelRuntime:
    deps: PromptRelayChannelDeps

    def require_messageable_channel(self, channel: PromptRelayChannel) -> PromptRelayChannel:
        if callable(getattr(channel, "send", None)):
            return channel
        raise DiscordMessageableChannelTypeError(channel)

    async def send_relay_chunks(
        self,
        channel: discord_stream_relay.RelayChannel,
        text: str,
    ) -> None:
        _ = await self.deps.send_chunks(self.require_messageable_channel(channel), text)

    async def send_relay_interactive_prompt(
        self,
        channel: discord_stream_relay.RelayChannel,
        target_thread_id: str,
        target_ref: str,
        state: str,
        prompt: str,
        options: discord_stream_relay.InteractiveNoticeOptions,
    ) -> None:
        await self.deps.send_interactive_prompt(
            self.require_messageable_channel(channel),
            target_thread_id,
            target_ref,
            state,
            prompt,
            _normalize_interactive_options(options),
        )

    def format_relay_log_text_len(self, text: str) -> str:
        return str(self.deps.format_log_text_len(text))

    def format_log_text_len_as_text(self, value: str | None) -> str:
        return str(self.deps.format_log_text_len(value))

    async def send_prompt_chunks(
        self,
        channel: PromptRelayChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        _ = await self.deps.send_chunks(
            self.require_messageable_channel(channel),
            content,
            context=context or "send_chunks",
        )


def _normalize_interactive_options(
    options: discord_stream_relay.InteractiveNoticeOptions,
) -> list[tuple[str, str]]:
    return [(str(label), str(value)) for label, value in _iter_option_pairs(options)]


def _iter_option_pairs(
    options: discord_stream_relay.InteractiveNoticeOptions,
) -> Sequence[tuple[str, str]]:
    return list(options)

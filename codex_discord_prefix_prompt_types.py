from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

__all__ = [
    "AuthorLike",
    "ChannelLike",
    "HandlePlainAskFunc",
    "MessageLike",
    "PrefixPromptCommandDeps",
    "SendChunksFunc",
]


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class AuthorLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...

    @property
    def author(self) -> AuthorLike:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int | None]:
        ...


class HandlePlainAskFunc(Protocol):
    def __call__(
        self,
        message: MessageLike,
        prompt: str,
        *,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixPromptCommandDeps:
    send_chunks: SendChunksFunc
    handle_plain_ask: HandlePlainAskFunc
    get_mirrored_codex_thread_id: Callable[[int], str | None]
    describe_mirrored_project_channel: Callable[[int], str]
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str], str]
    format_discord_command_label: Callable[[str], str]

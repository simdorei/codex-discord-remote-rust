from __future__ import annotations

import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

EMPTY_CONTENT_NOTICE_TEXT = (
    "I received a Discord message, but Discord did not provide the text content. "
    "Use `/ask` or enable the bot's Message Content Intent in the Discord developer portal."
)
EMPTY_CONTENT_NOTICE_COOLDOWN_SECONDS = 300.0
EMPTY_CONTENT_NOTICE_LAST_SENT: dict[int, float] = {}


class EmptyContentChannel(Protocol):
    @property
    def id(self) -> int | None: ...


class EmptyContentMessage(Protocol):
    @property
    def channel(self) -> EmptyContentChannel: ...


MessageHasNonTextPayloadFunc: TypeAlias = Callable[[EmptyContentMessage], bool]
SendChunksFunc: TypeAlias = Callable[[EmptyContentChannel, str], Awaitable[int]]
LogFunc: TypeAlias = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class EmptyContentNoticeDeps:
    message_has_non_text_payload: MessageHasNonTextPayloadFunc
    last_sent: dict[int, float]
    cooldown_seconds: float
    send_chunks: SendChunksFunc
    log_line: LogFunc


def make_empty_content_notice_deps(
    *,
    message_has_non_text_payload: MessageHasNonTextPayloadFunc,
    send_chunks: SendChunksFunc,
    log_line: LogFunc,
    last_sent: dict[int, float] | None = None,
    cooldown_seconds: float = EMPTY_CONTENT_NOTICE_COOLDOWN_SECONDS,
) -> EmptyContentNoticeDeps:
    return EmptyContentNoticeDeps(
        message_has_non_text_payload=message_has_non_text_payload,
        last_sent=EMPTY_CONTENT_NOTICE_LAST_SENT if last_sent is None else last_sent,
        cooldown_seconds=cooldown_seconds,
        send_chunks=send_chunks,
        log_line=log_line,
    )


def should_send_empty_content_notice(
    channel_id: int | None,
    *,
    last_sent: dict[int, float],
    cooldown_seconds: float,
    now: float | None = None,
) -> bool:
    if not channel_id:
        return False
    current = time.monotonic() if now is None else now
    normalized_channel_id = int(channel_id)
    previous = last_sent.get(normalized_channel_id)
    if previous is not None and current - previous < cooldown_seconds:
        return False
    last_sent[normalized_channel_id] = current
    return True


async def maybe_send_empty_content_notice(
    message: EmptyContentMessage,
    *,
    deps: EmptyContentNoticeDeps,
) -> None:
    channel = message.channel
    channel_id = channel.id
    if deps.message_has_non_text_payload(message):
        deps.log_line(f"empty_content_notice_skipped reason=non_text_payload chat={channel_id or '-'}")
        return
    if not should_send_empty_content_notice(
        channel_id,
        last_sent=deps.last_sent,
        cooldown_seconds=deps.cooldown_seconds,
    ):
        deps.log_line(f"empty_content_notice_skipped reason=cooldown chat={channel_id or '-'}")
        return
    _ = await deps.send_chunks(channel, EMPTY_CONTENT_NOTICE_TEXT)
    deps.log_line(f"empty_content_notice_sent chat={channel_id or '-'}")

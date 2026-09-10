from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

QUEUE_COMMANDS = {"retract", "unqueue"}

QueueRetractResultValue: TypeAlias = int | bool | str
QueueRetractResult: TypeAlias = dict[str, QueueRetractResultValue]


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
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


class RetractQueuedAskFunc(Protocol):
    def __call__(
        self,
        *,
        channel_id: int | None,
        user_id: int | None,
        ref: str | None,
    ) -> Awaitable[tuple[str, QueueRetractResult]]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixQueueCommandDeps:
    send_chunks: SendChunksFunc
    retract_queued_ask_for_request: RetractQueuedAskFunc
    log_line: Callable[[str], None]


async def handle_prefix_queue_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixQueueCommandDeps,
) -> bool:
    if command not in QUEUE_COMMANDS:
        return False
    try:
        response, _result = await deps.retract_queued_ask_for_request(
            channel_id=message.channel.id,
            user_id=message.author.id,
            ref=arg or None,
        )
    except RuntimeError as exc:
        deps.log_line("queue_retract_failed\n" + traceback.format_exc())
        _ = await deps.send_chunks(message.channel, f"Queue retract failed\n\nERROR: {exc}")
        return True
    _ = await deps.send_chunks(message.channel, response)
    return True

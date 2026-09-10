from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol


class ChannelLike(Protocol):
    @property
    def id(self) -> int: ...


class AuthorLike(Protocol):
    @property
    def id(self) -> int: ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike: ...

    @property
    def author(self) -> AuthorLike: ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]: ...


class RecoverResidentThreadFunc(Protocol):
    def __call__(self, channel_id: int, ref: str | None) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class PrefixResumeCommandDeps:
    send_chunks: SendChunksFunc
    recover_resident_thread_for_request: RecoverResidentThreadFunc
    log_line: Callable[[str], None]


async def handle_prefix_resume_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixResumeCommandDeps,
) -> bool:
    if command != "resume":
        return False
    try:
        response = await deps.recover_resident_thread_for_request(
            message.channel.id,
            arg or None,
        )
    except (OSError, RuntimeError, ValueError) as exc:
        deps.log_line("resident_thread_resume_failed\n" + traceback.format_exc())
        _ = await deps.send_chunks(
            message.channel,
            f"Resume failed\n\nERROR: {exc}\n\nNo prompt was resent.",
            context="prefix_resume_failed",
        )
        return True
    _ = await deps.send_chunks(message.channel, response, context="prefix_resume")
    return True

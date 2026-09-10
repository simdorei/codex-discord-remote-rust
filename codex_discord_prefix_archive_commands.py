from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

DELETE_ARCHIVE_COMMAND = "delete_archive"


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixArchiveCommandDeps:
    send_chunks: SendChunksFunc
    run_bridge_command: Callable[[list[str]], tuple[int, str]]


async def handle_prefix_archive_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixArchiveCommandDeps,
) -> bool:
    if command != DELETE_ARCHIVE_COMMAND:
        return False
    if not arg:
        _ = await deps.send_chunks(
            message.channel,
            "Usage: !delete_archive <ref>",
            context="prefix_delete_archive_usage",
        )
        return True
    exit_code, output = await asyncio.to_thread(deps.run_bridge_command, ["delete_archive", arg])
    prefix = "Delete archive preview" if exit_code == 0 else f"Delete archive failed (exit {exit_code})"
    _ = await deps.send_chunks(
        message.channel,
        f"{prefix}\n\n{output or '(no output)'}\n\nTo actually delete it, run `!confirm_delete_archive <thread_id>`.",
    )
    return True

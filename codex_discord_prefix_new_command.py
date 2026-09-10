from __future__ import annotations

from collections.abc import Awaitable
from dataclasses import dataclass
from typing import Protocol

NEW_COMMAND = "new"


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...


class BotLike(Protocol):
    ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


class RunDiscordNewThreadFunc(Protocol):
    def __call__(self, bot: BotLike, channel_id: int | None, prompt: str) -> Awaitable[tuple[int, str]]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixNewCommandDeps:
    send_chunks: SendChunksFunc
    run_discord_new_thread: RunDiscordNewThreadFunc


async def handle_prefix_new_command(
    command: str,
    arg: str,
    message: MessageLike,
    bot: BotLike,
    *,
    deps: PrefixNewCommandDeps,
) -> bool:
    if command != NEW_COMMAND:
        return False
    if not arg:
        _ = await deps.send_chunks(message.channel, "Usage: !new <prompt>", context="prefix_new_usage")
        return True
    _exit_code, output = await deps.run_discord_new_thread(bot, message.channel.id, arg)
    _ = await deps.send_chunks(message.channel, output)
    return True

from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

QA_COMMAND = "qa"
BotT = TypeVar("BotT")
BotT_contra = TypeVar("BotT_contra", contravariant=True)


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


class RunDiscordButtonQaFunc(Protocol[BotT_contra]):
    def __call__(self, bot: BotT_contra, message: MessageLike) -> Awaitable[str]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixQaCommandDeps(Generic[BotT]):
    send_chunks: SendChunksFunc
    qa_commands_enabled: Callable[[], bool]
    run_discord_button_qa: RunDiscordButtonQaFunc[BotT]
    log_line: Callable[[str], None]


async def handle_prefix_qa_command(
    command: str,
    arg: str,
    message: MessageLike,
    bot: BotT,
    *,
    deps: PrefixQaCommandDeps[BotT],
) -> bool:
    if command != QA_COMMAND:
        return False
    if not deps.qa_commands_enabled():
        _ = await deps.send_chunks(
            message.channel,
            "Discord QA commands are disabled. Set DISCORD_ENABLE_QA_COMMANDS=1 to enable them.",
            context="prefix_qa_disabled",
        )
        return True
    subcommand = (arg.strip() or "buttons").lower()
    if subcommand not in {"buttons", "button"}:
        _ = await deps.send_chunks(message.channel, "Usage: !qa buttons", context="prefix_qa_usage")
        return True
    _ = await deps.send_chunks(message.channel, "Discord button QA started.", context="prefix_qa_start")
    try:
        output = await deps.run_discord_button_qa(bot, message)
        _ = await deps.send_chunks(message.channel, output)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        deps.log_line("button_qa_failed\n" + traceback.format_exc())
        _ = await deps.send_chunks(message.channel, f"Discord button QA failed\n\nERROR: {exc}")
    return True

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_commands as discord_commands

IDENTITY_COMMANDS = {"chatid", "whoami"}
WHERE_COMMANDS = {"where", "map"}
CONTEXT_COMMANDS = {"context", "ctx"}
USAGE_COMMANDS = {"usage", "quota", "limit"}
RUNNERS_COMMANDS = {"runners", "queues"}
RESOURCE_COMMANDS = {"resources", "system"}


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class AuthorLike(Protocol):
    @property
    def id(self) -> int:
        ...


class GuildLike(Protocol):
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

    @property
    def guild(self) -> GuildLike | None:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


class BuildContextMessageFunc(Protocol):
    def __call__(
        self,
        channel_id: int | None = None,
        *,
        all_threads: bool = False,
        limit: int = 10,
    ) -> str:
        ...


class BuildContextRefreshMessageFunc(Protocol):
    def __call__(self, channel_id: int | None = None, *, limit: int = 10) -> str:
        ...


class BuildSystemResourcesMessageFunc(Protocol):
    def __call__(self) -> Awaitable[str]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixStatusCommandDeps:
    send_chunks: SendChunksFunc
    build_where_message: Callable[[int | None], str]
    build_context_message: BuildContextMessageFunc
    build_context_refresh_message: BuildContextRefreshMessageFunc
    clamp_context_refresh_limit: Callable[[str], int]
    build_weekly_usage_message: Callable[[int], str]
    build_runners_message: Callable[[], Awaitable[str]]
    build_system_resources_message: BuildSystemResourcesMessageFunc


async def handle_prefix_status_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixStatusCommandDeps,
) -> bool:
    if command in IDENTITY_COMMANDS:
        _ = await deps.send_chunks(message.channel, _build_identity_message(message))
        return True
    if command in WHERE_COMMANDS:
        _ = await deps.send_chunks(message.channel, deps.build_where_message(message.channel.id))
        return True
    if command in CONTEXT_COMMANDS:
        await _handle_context(arg, message, deps=deps)
        return True
    if command in USAGE_COMMANDS:
        await _handle_usage(arg, message, deps=deps)
        return True
    if command in RUNNERS_COMMANDS:
        _ = await deps.send_chunks(message.channel, await deps.build_runners_message())
        return True
    if command in RESOURCE_COMMANDS:
        _ = await deps.send_chunks(message.channel, await deps.build_system_resources_message())
        return True
    return False


def _build_identity_message(message: MessageLike) -> str:
    guild = message.guild
    guild_id: int | str = "-" if guild is None else guild.id
    channel_name = getattr(message.channel, "name", "-")
    return "\n".join(
        [
            "Discord identity",
            f"guild_id: {guild_id}",
            f"channel_id: {message.channel.id}",
            f"user_id: {message.author.id}",
            f"channel_name: {channel_name}",
            "",
            "Copy into .env if needed:",
            f"DISCORD_ALLOWED_CHANNEL_IDS={message.channel.id}",
            f"DISCORD_ALLOWED_USER_IDS={message.author.id}",
        ]
    )


async def _handle_context(
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixStatusCommandDeps,
) -> None:
    normalized_arg = arg.lower().strip()
    context_args = normalized_arg.split()
    if context_args and context_args[0] in {"refresh", "recent"}:
        limit_arg = context_args[1] if len(context_args) > 1 else ""
        _ = await deps.send_chunks(
            message.channel,
            deps.build_context_refresh_message(
                message.channel.id,
                limit=deps.clamp_context_refresh_limit(limit_arg),
            ),
        )
        return
    if normalized_arg in {"all", "*"}:
        _ = await deps.send_chunks(
            message.channel,
            deps.build_context_message(message.channel.id, all_threads=True, limit=20),
        )
        return
    _ = await deps.send_chunks(message.channel, deps.build_context_message(message.channel.id))


async def _handle_usage(
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixStatusCommandDeps,
) -> None:
    usage_action = discord_commands.parse_usage_days(arg)
    if usage_action.usage:
        _ = await deps.send_chunks(message.channel, usage_action.usage, context="prefix_usage_help")
        return
    days = int(usage_action.limit or 7)
    output = await asyncio.to_thread(deps.build_weekly_usage_message, days)
    _ = await deps.send_chunks(message.channel, output)

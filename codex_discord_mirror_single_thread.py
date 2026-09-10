from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import sys
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar, override

import discord

import codex_discord_mirror_coordination as discord_mirror_coordination
from codex_thread_models import ThreadInfo

BotT = TypeVar("BotT")
ExceptionTypes = tuple[type[BaseException], ...]


@dataclass(frozen=True, slots=True)
class PreferredMirrorProjectChannelUnavailableError(RuntimeError):
    channel_id: int
    cause_type: str
    cause_message: str

    @override
    def __str__(self) -> str:
        return (
            f"Preferred mirror project channel {self.channel_id} "
            + f"is unavailable: {self.cause_type}: {self.cause_message}"
        )


@dataclass(frozen=True, slots=True)
class PreferredMirrorProjectChannelTypeError(RuntimeError):
    channel_id: int
    actual_type: str

    @override
    def __str__(self) -> str:
        return f"Preferred mirror project channel {self.channel_id} is {self.actual_type}, not TextChannel."


@dataclass(frozen=True, slots=True)
class MirrorSingleThreadDeps(Generic[BotT]):
    get_mirror_guild: Callable[[BotT], Awaitable[discord.Guild]]
    get_or_create_mirror_category: Callable[[discord.Guild], Awaitable[discord.CategoryChannel]]
    choose_thread: Callable[[str, str | None], ThreadInfo]
    get_project_key: Callable[[ThreadInfo], str]
    get_project_name: Callable[[ThreadInfo], str]
    upsert_mirror_project: Callable[[str, str, int], None]
    get_or_create_project_channel: Callable[
        [discord.Guild, discord.CategoryChannel, str, str],
        Awaitable[discord.TextChannel],
    ]
    get_or_create_thread_channel: Callable[[ThreadInfo, str, discord.TextChannel], Awaitable[discord.Thread]]
    delivery_exceptions: ExceptionTypes
    log: Callable[[str], None]


@discord_mirror_coordination.serialize_mirror_mutation
async def mirror_single_codex_thread(
    bot: BotT,
    thread_id: str,
    *,
    preferred_project_channel_id: int | None = None,
    deps: MirrorSingleThreadDeps[BotT],
) -> discord.Thread:
    guild = await deps.get_mirror_guild(bot)
    category = await deps.get_or_create_mirror_category(guild)
    codex_thread = await asyncio.to_thread(deps.choose_thread, thread_id, None)
    project_key = deps.get_project_key(codex_thread)
    project_name = deps.get_project_name(codex_thread)
    project_channel = None
    if preferred_project_channel_id is not None:
        candidate = guild.get_channel(int(preferred_project_channel_id))
        if not isinstance(candidate, discord.TextChannel):
            try:
                fetched = await guild.fetch_channel(int(preferred_project_channel_id))
            except deps.delivery_exceptions:
                exc = sys.exception() or RuntimeError("")
                raise PreferredMirrorProjectChannelUnavailableError(
                    channel_id=preferred_project_channel_id,
                    cause_type=type(exc).__name__,
                    cause_message=str(exc),
                ) from exc
            if not isinstance(fetched, discord.TextChannel):
                raise PreferredMirrorProjectChannelTypeError(
                    channel_id=preferred_project_channel_id,
                    actual_type=type(fetched).__name__,
                )
            candidate = fetched
        project_channel = candidate
        deps.upsert_mirror_project(project_key, project_name, int(project_channel.id))
        deps.log(
            f"single_thread_mirror_preferred_channel codex_thread={thread_id} "
            + f"project_channel={project_channel.id}"
        )
    if project_channel is None:
        project_channel = await deps.get_or_create_project_channel(guild, category, project_key, project_name)
    return await deps.get_or_create_thread_channel(codex_thread, project_key, project_channel)

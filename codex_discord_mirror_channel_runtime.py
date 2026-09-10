from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import discord

import codex_discord_mirror_channels as discord_mirror_channels
import codex_discord_mirror_thread_channels as discord_mirror_thread_channels
from codex_thread_models import ThreadInfo

FetchFailureTypes = tuple[type[Exception], ...]
GetDbPathFunc = Callable[[], Path]
NormalizeProjectKeyFunc = Callable[[str | None], str]
ProjectKeysMatchFunc = Callable[[str | None, str | None], bool]
GetThreadUiNameFunc = Callable[[str, ThreadInfo], str | None]
GetThreadUiNameFactory = Callable[[], GetThreadUiNameFunc]
LogFunc = Callable[[str], None]


class MirrorRuntimeBot(Protocol):
    @property
    def guild_id(self) -> int | None: ...

    @property
    def guilds(self) -> Sequence[discord.Guild]: ...

    def get_guild(self, guild_id: int) -> discord.Guild | None: ...


class MirrorGuildUnavailableError(RuntimeError):
    def __str__(self) -> str:
        return "Discord guild is not available yet."


@dataclass(frozen=True, slots=True)
class MirrorChannelRuntime:
    get_db_path: GetDbPathFunc
    normalize_project_key: NormalizeProjectKeyFunc
    project_keys_match: ProjectKeysMatchFunc
    get_thread_ui_name: GetThreadUiNameFactory
    log: LogFunc
    fetch_failure_types: FetchFailureTypes = (Exception,)

    async def get_mirror_guild(self, bot: MirrorRuntimeBot) -> discord.Guild:
        guild = bot.get_guild(bot.guild_id) if bot.guild_id else (bot.guilds[0] if bot.guilds else None)
        if guild is None:
            raise MirrorGuildUnavailableError()
        return guild

    async def get_or_create_mirror_category(self, guild: discord.Guild) -> discord.CategoryChannel:
        for category in guild.categories:
            if category.name == "Codex":
                return category
        return await guild.create_category("Codex", reason="Codex mirror setup")

    def mirror_channel_deps(self) -> discord_mirror_channels.MirrorChannelDeps:
        return discord_mirror_channels.MirrorChannelDeps(
            db_path=self.get_db_path(),
            normalize_project_key=self.normalize_project_key,
            project_keys_match=self.project_keys_match,
            get_thread_ui_name=self.get_thread_ui_name(),
            log=self.log,
            fetch_failure_types=self.fetch_failure_types,
        )

    def upsert_mirror_project(self, project_key: str, project_name: str, channel_id: int) -> None:
        discord_mirror_channels.upsert_mirror_project(
            project_key,
            project_name,
            channel_id,
            deps=self.mirror_channel_deps(),
        )

    def upsert_mirror_thread(
        self,
        codex_thread: ThreadInfo,
        project_key: str,
        thread_name: str,
        project_channel_id: int,
        discord_thread_id: int,
    ) -> None:
        discord_mirror_channels.upsert_mirror_thread(
            codex_thread,
            project_key,
            thread_name,
            project_channel_id,
            discord_thread_id,
            deps=self.mirror_channel_deps(),
        )

    async def ensure_mirror_project_channel(
        self,
        guild: discord.Guild,
        channel: discord.TextChannel,
        project_key: str,
        project_name: str,
    ) -> discord.TextChannel:
        return await discord_mirror_channels.ensure_mirror_project_channel(
            guild,
            channel,
            project_key,
            project_name,
            deps=self.mirror_channel_deps(),
        )

    async def get_or_create_project_channel(
        self,
        guild: discord.Guild,
        category: discord.CategoryChannel,
        project_key: str,
        project_name: str,
    ) -> discord.TextChannel:
        return await discord_mirror_channels.get_or_create_project_channel(
            guild,
            category,
            project_key,
            project_name,
            deps=self.mirror_channel_deps(),
        )

    async def get_or_create_thread_channel(
        self,
        codex_thread: ThreadInfo,
        project_key: str,
        project_channel: discord.TextChannel,
    ) -> discord.Thread:
        return await discord_mirror_thread_channels.get_or_create_thread_channel(
            codex_thread,
            project_key,
            project_channel,
            deps=self.mirror_channel_deps(),
        )

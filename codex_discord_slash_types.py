from __future__ import annotations

from collections.abc import Awaitable, Callable, Coroutine, Mapping
from dataclasses import dataclass
from typing import Protocol, TypeAlias

import discord

from codex_model_catalog import JsonValue

__all__ = [
    "BasicSlashCommandDeps",
    "BasicSlashInteraction",
    "ContextMessageBuilder",
    "ContextRefreshMessageBuilder",
    "InteractionBridgeRunner",
    "InteractionChunksSender",
    "InteractionResponse",
    "SlashCallback",
    "SlashCommandBot",
    "SlashCommandTree",
    "SlashInteraction",
    "SettingsModelCatalog",
    "SettingsModelCatalogLoader",
    "WeeklyUsageMessageBuilder",
]


SlashCallback = Callable[..., Coroutine[object, object, None]]


class SlashCommandTree(Protocol):
    def command(
        self,
        *,
        name: str,
        description: str,
    ) -> Callable[[SlashCallback], SlashCallback]: ...


class SlashCommandBot(Protocol):
    @property
    def tree(self) -> SlashCommandTree: ...


class InteractionResponse(Protocol):
    def defer(self, thinking: bool = False, **kwargs: bool) -> Awaitable[None]: ...


class SlashInteraction(Protocol):
    @property
    def channel_id(self) -> int | None: ...

    @property
    def response(self) -> InteractionResponse: ...


SettingsModelCatalog: TypeAlias = Mapping[str, JsonValue]
SettingsModelCatalogLoader: TypeAlias = Callable[[], SettingsModelCatalog]
BasicSlashInteraction: TypeAlias = SlashInteraction | discord.Interaction


class InteractionChunksSender(Protocol):
    def __call__(
        self,
        interaction: SlashInteraction,
        text: str,
        *,
        title: str,
    ) -> Awaitable[None]: ...


class InteractionBridgeRunner(Protocol):
    def __call__(
        self,
        interaction: SlashInteraction,
        argv: list[str],
        title: str,
    ) -> Awaitable[tuple[int, str]]: ...


class ContextMessageBuilder(Protocol):
    def __call__(
        self,
        channel_id: int | None,
        *,
        all_threads: bool = False,
        limit: int = 20,
    ) -> str: ...


class ContextRefreshMessageBuilder(Protocol):
    def __call__(self, channel_id: int | None, *, limit: int) -> str: ...


class WeeklyUsageMessageBuilder(Protocol):
    def __call__(self, *, days: int) -> str: ...


@dataclass(frozen=True, slots=True)
class BasicSlashCommandDeps:
    check_allowed: Callable[[BasicSlashInteraction], bool]
    send_not_allowed: Callable[[SlashInteraction], Awaitable[None]]
    send_chunks: InteractionChunksSender
    run_bridge: InteractionBridgeRunner
    build_help: Callable[[], str]
    build_where: Callable[[int | None], str]
    build_context: ContextMessageBuilder
    build_context_refresh: ContextRefreshMessageBuilder
    build_weekly_usage: WeeklyUsageMessageBuilder
    clamp_context_refresh_limit: Callable[[int], int]
    resolve_target_args: Callable[[int | None, str | None], list[str]]
    load_settings_model_catalog: SettingsModelCatalogLoader

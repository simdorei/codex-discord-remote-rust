from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, Protocol, TypeVar

import discord

import codex_discord_bridge_session_state as discord_bridge_session_state
import codex_discord_mirror_access as discord_mirror_access
import codex_discord_mirror_scope as discord_mirror_scope
import codex_discord_mirror_single_thread as discord_mirror_single_thread
import codex_discord_mirror_status_runtime_bridge as discord_mirror_status_runtime_bridge
import codex_discord_mirror_sync as discord_mirror_sync
from codex_thread_models import ThreadInfo


BotT = TypeVar("BotT", bound=discord_mirror_access.MirrorAccessBot)
BotContraT = TypeVar("BotContraT", bound=discord_mirror_access.MirrorAccessBot, contravariant=True)
ExceptionTypes = tuple[type[BaseException], ...]


class SyncCodexMirrorFunc(Protocol[BotContraT]):
    def __call__(self, bot: BotContraT, *, limit: int | None = None) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class MirrorRuntimeDeps(Generic[BotT]):
    get_db_path: Callable[[], Path]
    get_mirror_scope_bridge_module: Callable[[], discord_mirror_scope.MirrorScopeBridge]
    load_mirror_scope_threads: Callable[[int | None], list[ThreadInfo]]
    filter_threads_for_discord_channel: Callable[[list[ThreadInfo], int | None], list[ThreadInfo]]
    filter_mirrorable_threads: Callable[[list[ThreadInfo]], list[ThreadInfo]]
    filter_app_server_available_threads: Callable[[list[ThreadInfo]], list[ThreadInfo]]
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
    get_or_create_thread_channel: Callable[
        [ThreadInfo, str, discord.TextChannel],
        Awaitable[discord.Thread],
    ]
    get_mirrored_codex_thread_id: Callable[[int | None], str | None]
    get_mirror_project_for_channel: Callable[[int | None], tuple[str, str] | None]
    project_keys_match: Callable[[str, str], bool]
    refresh_session_state: Callable[[], discord_bridge_session_state.RefreshBridgeSessionState]
    sync_codex_mirror: SyncCodexMirrorFunc[BotT]
    get_bot_user_id: Callable[[BotT], int | None]
    init_mirror_db: Callable[[], None]
    get_mirror_status_bridge_module: discord_mirror_status_runtime_bridge.GetMirrorStatusBridgeFunc
    delivery_exceptions: ExceptionTypes
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class MirrorRuntime(Generic[BotT]):
    deps: MirrorRuntimeDeps[BotT]

    def load_mirror_scope_threads(self, limit: int | None = None) -> list[ThreadInfo]:
        return discord_mirror_scope.load_mirror_scope_threads(
            self.deps.get_mirror_scope_bridge_module(),
            limit,
        )

    def filter_threads_for_discord_channel(
        self,
        threads: list[ThreadInfo],
        channel_id: int | None,
    ) -> list[ThreadInfo]:
        return discord_mirror_scope.filter_threads_for_discord_channel(
            threads,
            channel_id,
            bridge_module=self.deps.get_mirror_scope_bridge_module(),
            get_mirrored_codex_thread_id=self.deps.get_mirrored_codex_thread_id,
            get_mirror_project_for_channel=self.deps.get_mirror_project_for_channel,
            project_keys_match=self.deps.project_keys_match,
            get_project_key=self.deps.get_project_key,
        )

    async def sync_codex_mirror(self, bot: BotT, *, limit: int | None = None) -> str:
        return await discord_mirror_sync.sync_codex_mirror(
            bot,
            limit=limit,
            deps=discord_mirror_sync.CodexMirrorSyncDeps(
                db_path=self.deps.get_db_path(),
                get_mirror_guild=self.deps.get_mirror_guild,
                get_or_create_mirror_category=self.deps.get_or_create_mirror_category,
                load_mirror_scope_threads=self.load_mirror_scope_threads,
                filter_mirrorable_threads=self.deps.filter_mirrorable_threads,
                filter_app_server_available_threads=self.deps.filter_app_server_available_threads,
                get_project_key=self.deps.get_project_key,
                get_project_name=self.deps.get_project_name,
                get_or_create_project_channel=self.deps.get_or_create_project_channel,
                get_or_create_thread_channel=self.deps.get_or_create_thread_channel,
                get_bot_user_id=self.deps.get_bot_user_id,
                log=self.deps.log,
            ),
        )

    def refresh_codex_bridge_session_state(
        self,
    ) -> discord_bridge_session_state.RefreshBridgeSessionState:
        return self.deps.refresh_session_state()

    async def refresh_discord_bridge_session(
        self,
        bot: BotT,
        *,
        limit: int | None = None,
    ) -> str:
        state = await asyncio.to_thread(self.refresh_codex_bridge_session_state)
        mirror_output = await self.deps.sync_codex_mirror(bot, limit=limit)
        return "\n".join(
            [
                "Discord bridge sync complete.",
                f"session_index_threads: {state['session_index_count']}",
                f"codex_threads: {state['thread_count']}",
                f"selected_action: {state['selected_action']}",
                f"selected_thread: {state['selected_ref']} ({state['selected_thread_id']})",
                f"selected_before: {state['selected_before']}",
                "",
                mirror_output,
            ]
        )

    async def mirror_single_codex_thread(
        self,
        bot: BotT,
        thread_id: str,
        *,
        preferred_project_channel_id: int | None = None,
    ) -> discord.Thread:
        return await discord_mirror_single_thread.mirror_single_codex_thread(
            bot,
            thread_id,
            preferred_project_channel_id=preferred_project_channel_id,
            deps=discord_mirror_single_thread.MirrorSingleThreadDeps(
                get_mirror_guild=self.deps.get_mirror_guild,
                get_or_create_mirror_category=self.deps.get_or_create_mirror_category,
                choose_thread=self.deps.choose_thread,
                get_project_key=self.deps.get_project_key,
                get_project_name=self.deps.get_project_name,
                upsert_mirror_project=self.deps.upsert_mirror_project,
                get_or_create_project_channel=self.deps.get_or_create_project_channel,
                get_or_create_thread_channel=self.deps.get_or_create_thread_channel,
                delivery_exceptions=self.deps.delivery_exceptions,
                log=self.deps.log,
            ),
        )

    def _status_runtime(self) -> discord_mirror_status_runtime_bridge.MirrorStatusRuntime:
        return discord_mirror_status_runtime_bridge.MirrorStatusRuntime(
            get_db_path=self.deps.get_db_path,
            init_mirror_db=self.deps.init_mirror_db,
            get_mirror_status_bridge_module=self.deps.get_mirror_status_bridge_module,
            load_mirror_scope_threads=self.load_mirror_scope_threads,
            filter_threads_for_discord_channel=self.deps.filter_threads_for_discord_channel,
            filter_mirrorable_threads=self.deps.filter_mirrorable_threads,
            filter_app_server_available_threads=self.deps.filter_app_server_available_threads,
            get_project_key=self.deps.get_project_key,
            get_project_name=self.deps.get_project_name,
        )

    def build_mirror_list(self, limit: int | None = None, *, channel_id: int | None = None) -> str:
        return self._status_runtime().build_mirror_list(limit, channel_id=channel_id)

    async def build_mirror_list_for_prefix(
        self,
        bot: BotT,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        return await self._status_runtime().build_mirror_list_for_prefix(
            bot,
            limit,
            channel_id=channel_id,
        )

    def build_mirror_check(
        self,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        return self._status_runtime().build_mirror_check(limit, channel_id=channel_id)

    async def build_mirror_check_for_prefix(
        self,
        bot: BotT,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        return await self._status_runtime().build_mirror_check_for_prefix(
            bot,
            limit,
            channel_id=channel_id,
        )

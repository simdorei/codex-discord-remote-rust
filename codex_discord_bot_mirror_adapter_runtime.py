from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import discord

import codex_discord_bridge_session_state as discord_bridge_session_state
import codex_discord_bridge_protocols as discord_bridge_protocols
import codex_discord_mirror_access as discord_mirror_access
import codex_discord_mirror_runtime as discord_mirror_runtime
import codex_discord_mirror_scope as discord_mirror_scope
import codex_discord_mirror_status_runtime_bridge as discord_mirror_status_runtime_bridge
from codex_thread_models import ThreadInfo
ModuleValue: TypeAlias = object


class BotUserLike(Protocol):
    @property
    def id(self) -> int: ...


class MirrorBotWithUser(discord_mirror_access.MirrorAccessBot, Protocol):
    @property
    def user(self) -> BotUserLike | None: ...


@dataclass(frozen=True, slots=True)
class BotMirrorAdapterRuntime:
    module: ModuleType

    def make_mirror_runtime(
        self,
    ) -> discord_mirror_runtime.MirrorRuntime[discord_mirror_access.MirrorAccessBot]:
        return discord_mirror_runtime.MirrorRuntime(
            discord_mirror_runtime.MirrorRuntimeDeps(
                get_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
                get_mirror_scope_bridge_module=lambda: cast(
                    Callable[[], discord_mirror_scope.MirrorScopeBridge],
                    self._module_func("get_mirror_scope_bridge_module"),
                )(),
                load_mirror_scope_threads=lambda limit=None: cast(
                    Callable[[int | None], list[ThreadInfo]],
                    self._module_func("load_mirror_scope_threads"),
                )(limit),
                filter_threads_for_discord_channel=lambda threads, channel_id: cast(
                    Callable[[list[ThreadInfo], int | None], list[ThreadInfo]],
                    self._module_func("filter_threads_for_discord_channel"),
                )(threads, channel_id),
                filter_mirrorable_threads=lambda threads: cast(
                    Callable[[list[ThreadInfo]], list[ThreadInfo]],
                    self._module_func("filter_mirrorable_threads"),
                )(threads),
                filter_app_server_available_threads=lambda threads: cast(
                    Callable[[list[ThreadInfo]], list[ThreadInfo]],
                    self._module_func("filter_app_server_available_threads"),
                )(threads),
                get_mirror_guild=lambda bot: cast(
                    Callable[[discord_mirror_access.MirrorAccessBot], Awaitable[discord.Guild]],
                    self._module_func("get_mirror_guild"),
                )(bot),
                get_or_create_mirror_category=lambda guild: cast(
                    Callable[[discord.Guild], Awaitable[discord.CategoryChannel]],
                    self._module_func("get_or_create_mirror_category"),
                )(guild),
                choose_thread=lambda thread_id, fallback: cast(
                    Callable[[str, str | None], ThreadInfo],
                    getattr(self.module, "BRIDGE_SESSION_STATE").choose_thread,
                )(thread_id, fallback),
                get_project_key=lambda thread: cast(Callable[[ThreadInfo], str], self._module_func("get_project_key"))(thread),
                get_project_name=lambda thread: cast(Callable[[ThreadInfo], str], self._module_func("get_project_name"))(thread),
                upsert_mirror_project=lambda project_key, project_name, channel_id: cast(
                    Callable[[str, str, int], None],
                    self._module_func("upsert_mirror_project"),
                )(project_key, project_name, channel_id),
                get_or_create_project_channel=lambda guild, category, project_key, project_name: cast(
                    Callable[
                        [discord.Guild, discord.CategoryChannel, str, str],
                        Awaitable[discord.TextChannel],
                    ],
                    self._module_func("get_or_create_project_channel"),
                )(guild, category, project_key, project_name),
                get_or_create_thread_channel=lambda thread, project_key, project_channel: cast(
                    Callable[[ThreadInfo, str, discord.TextChannel], Awaitable[discord.Thread]],
                    self._module_func("get_or_create_thread_channel"),
                )(thread, project_key, project_channel),
                get_mirrored_codex_thread_id=lambda channel_id: cast(
                    Callable[[int | None], str | None],
                    self._module_func("get_mirrored_codex_thread_id"),
                )(channel_id),
                get_mirror_project_for_channel=lambda channel_id: cast(
                    Callable[[int | None], tuple[str, str] | None],
                    self._module_func("get_mirror_project_for_channel"),
                )(channel_id),
                project_keys_match=lambda left, right: cast(
                    Callable[[str, str], bool],
                    self._module_func("project_keys_match"),
                )(left, right),
                refresh_session_state=self.refresh_session_state,
                sync_codex_mirror=self.sync_codex_mirror,
                get_bot_user_id=self.get_bot_user_id,
                init_mirror_db=lambda: cast(Callable[[], None], self._module_func("init_mirror_db"))(),
                get_mirror_status_bridge_module=lambda: cast(
                    discord_mirror_status_runtime_bridge.GetMirrorStatusBridgeFunc,
                    self._module_func("get_mirror_status_bridge_module"),
                )(),
                delivery_exceptions=cast(
                    tuple[type[BaseException], ...],
                    getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS"),
                ),
                log=lambda message: cast(Callable[[str], None], self._module_func("log_line"))(message),
            )
        )

    def get_bot_user_id(self, bot: discord_mirror_access.MirrorAccessBot) -> int | None:
        user = cast(MirrorBotWithUser, bot).user
        return user.id if user else None

    def refresh_session_state(self) -> discord_bridge_session_state.RefreshBridgeSessionState:
        return discord_bridge_session_state.refresh_codex_bridge_session_state(
            cast(
                discord_bridge_protocols.CodexBridgeSessionStateModule,
                getattr(self.module, "BRIDGE_SESSION_STATE"),
            ),
        )

    def sync_codex_mirror(
        self,
        bot: discord_mirror_access.MirrorAccessBot,
        *,
        limit: int | None = None,
    ) -> Awaitable[str]:
        sync_mirror = cast(
            discord_mirror_runtime.SyncCodexMirrorFunc[discord_mirror_access.MirrorAccessBot],
            self._module_func("sync_codex_mirror"),
        )
        return sync_mirror(bot, limit=limit)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))

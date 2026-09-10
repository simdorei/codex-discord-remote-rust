from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_mirror_access as discord_mirror_access
import codex_discord_mirror_status as discord_mirror_status
import codex_discord_mirror_status_runtime as discord_mirror_status_runtime
from codex_thread_models import ThreadInfo

GetDbPathFunc = Callable[[], Path]
InitMirrorDbFunc = Callable[[], None]
GetMirrorStatusBridgeFunc = Callable[[], discord_mirror_status_runtime.MirrorStatusBridge]
LoadMirrorScopeThreadsFunc = Callable[[int | None], list[ThreadInfo]]
FilterThreadsForDiscordChannelFunc = Callable[[list[ThreadInfo], int | None], list[ThreadInfo]]
FilterMirrorableThreadsFunc = Callable[[list[ThreadInfo]], list[ThreadInfo]]
FilterAppServerAvailableThreadsFunc = Callable[[list[ThreadInfo]], list[ThreadInfo]]
GetProjectKeyFunc = Callable[[ThreadInfo], str]
GetProjectNameFunc = Callable[[ThreadInfo], str]


@dataclass(frozen=True, slots=True)
class MirrorStatusRuntime:
    get_db_path: GetDbPathFunc
    init_mirror_db: InitMirrorDbFunc
    get_mirror_status_bridge_module: GetMirrorStatusBridgeFunc
    load_mirror_scope_threads: LoadMirrorScopeThreadsFunc
    filter_threads_for_discord_channel: FilterThreadsForDiscordChannelFunc
    filter_mirrorable_threads: FilterMirrorableThreadsFunc
    filter_app_server_available_threads: FilterAppServerAvailableThreadsFunc
    get_project_key: GetProjectKeyFunc
    get_project_name: GetProjectNameFunc

    def deps(self) -> discord_mirror_status_runtime.MirrorStatusRuntimeDeps:
        return discord_mirror_status_runtime.MirrorStatusRuntimeDeps(
            db_path=self.get_db_path(),
            init_mirror_db=self.init_mirror_db,
            get_mirror_status_bridge_module=self.get_mirror_status_bridge_module,
            load_mirror_scope_threads=self.load_mirror_scope_threads,
            filter_threads_for_discord_channel=self.filter_threads_for_discord_channel,
            filter_mirrorable_threads=self.filter_mirrorable_threads,
            filter_app_server_available_threads=self.filter_app_server_available_threads,
            get_project_key=self.get_project_key,
            get_project_name=self.get_project_name,
        )

    def build_mirror_list(self, limit: int | None = None, *, channel_id: int | None = None) -> str:
        return discord_mirror_status_runtime.build_mirror_list(
            limit,
            channel_id=channel_id,
            deps=self.deps(),
        )

    async def build_mirror_list_for_prefix(
        self,
        bot: discord_mirror_access.MirrorAccessBot,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        return await discord_mirror_status_runtime.build_mirror_list_for_prefix(
            bot,
            limit,
            channel_id=channel_id,
            deps=self.deps(),
        )

    def build_mirror_check(
        self,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
        access_statuses: discord_mirror_status.MirrorAccessStatusMap | None = None,
    ) -> str:
        return discord_mirror_status_runtime.build_mirror_check(
            limit,
            channel_id=channel_id,
            access_statuses=access_statuses,
            deps=self.deps(),
        )

    async def build_mirror_check_for_prefix(
        self,
        bot: discord_mirror_access.MirrorAccessBot,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        return await discord_mirror_status_runtime.build_mirror_check_for_prefix(
            bot,
            limit,
            channel_id=channel_id,
            deps=self.deps(),
        )

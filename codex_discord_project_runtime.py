from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_project_types as discord_project_types
import codex_discord_projects as discord_projects
import codex_discord_store as discord_store
from codex_thread_models import ThreadInfo

GetDbPathFunc = Callable[[], Path]
GetProjectBridgeModuleFunc = Callable[[], discord_project_types.BridgeProjectModule]
InitMirrorDbFunc = Callable[[], None]


@dataclass(frozen=True, slots=True)
class ProjectRuntime:
    get_db_path: GetDbPathFunc
    get_project_bridge_module: GetProjectBridgeModuleFunc
    projectless_chat_key: str
    init_mirror_db_func: InitMirrorDbFunc
    get_mirrored_codex_thread_id_func: discord_project_types.GetMirroredCodexThreadId
    get_thread_cwd_func: discord_project_types.GetThreadCwd
    get_mirror_project_for_channel_func: discord_project_types.GetMirrorProjectForChannel
    project_keys_match_func: discord_project_types.ProjectKeysMatch

    def get_project_key(self, thread: ThreadInfo) -> str:
        return discord_projects.get_project_key(
            thread,
            bridge_module=self.get_project_bridge_module(),
            projectless_chat_key=self.projectless_chat_key,
        )

    def normalize_project_key(self, project_key: str | None) -> str:
        return discord_projects.normalize_project_key(
            project_key,
            bridge_module=self.get_project_bridge_module(),
            projectless_chat_key=self.projectless_chat_key,
        )

    def get_project_name(self, thread: ThreadInfo) -> str:
        return discord_projects.get_project_name(thread, bridge_module=self.get_project_bridge_module())

    def filter_mirrorable_threads(self, threads: list[ThreadInfo]) -> list[ThreadInfo]:
        return discord_projects.filter_mirrorable_threads(
            threads,
            bridge_module=self.get_project_bridge_module(),
            projectless_chat_key=self.projectless_chat_key,
        )

    def get_mirrored_codex_thread_id(self, discord_channel_id: int | None) -> str | None:
        return discord_store.get_mirrored_codex_thread_id(self.get_db_path(), discord_channel_id)

    def persist_inbound_mirror_thread_channel(self, target_thread_id: str, discord_thread_id: int) -> None:
        _ = discord_store.update_mirror_thread_discord_thread_id(
            self.get_db_path(),
            target_thread_id,
            int(discord_thread_id),
        )

    def describe_mirrored_project_channel(self, discord_channel_id: int | None) -> str:
        return discord_store.describe_mirrored_project_channel(self.get_db_path(), discord_channel_id)

    def get_mirror_project_for_channel(self, discord_channel_id: int | None) -> tuple[str, str] | None:
        return discord_store.get_mirror_project_for_channel(self.get_db_path(), discord_channel_id)

    def get_thread_cwd(self, thread_id: str | None) -> str | None:
        return discord_projects.get_thread_cwd(thread_id, bridge_module=self.get_project_bridge_module())

    def resolve_discord_new_thread_cwd(self, discord_channel_id: int | None) -> str | None:
        return discord_projects.resolve_discord_new_thread_cwd(
            discord_channel_id,
            bridge_module=self.get_project_bridge_module(),
            projectless_chat_key=self.projectless_chat_key,
            get_mirrored_codex_thread_id_func=self.get_mirrored_codex_thread_id_func,
            get_thread_cwd_func=self.get_thread_cwd_func,
            get_mirror_project_for_channel_func=self.get_mirror_project_for_channel_func,
            find_projectless_new_chat_cwd_func=discord_projects.find_projectless_new_chat_cwd,
        )

    def project_keys_match(self, left: str | None, right: str | None) -> bool:
        return discord_projects.project_keys_match(
            left,
            right,
            bridge_module=self.get_project_bridge_module(),
            projectless_chat_key=self.projectless_chat_key,
        )

    def resolve_discord_new_thread_project_channel_id(
        self,
        discord_channel_id: int | None,
        project_key: str | None,
    ) -> int | None:
        return discord_projects.resolve_discord_new_thread_project_channel_id(
            discord_channel_id,
            project_key,
            db_path=self.get_db_path(),
            init_mirror_db_func=self.init_mirror_db_func,
            project_keys_match_func=self.project_keys_match_func,
        )

    def is_mirrored_channel_id(self, discord_channel_id: int | None) -> bool:
        return discord_store.is_mirrored_channel_id(self.get_db_path(), discord_channel_id)

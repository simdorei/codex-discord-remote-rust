"""Project/path helpers for Discord Codex mirrors."""

from __future__ import annotations

from pathlib import Path

import codex_discord_project_channels as project_channels
from codex_discord_project_paths import (
    filter_mirrorable_threads as filter_mirrorable_threads,
    find_projectless_new_chat_cwd as find_projectless_new_chat_cwd,
    get_project_key as get_project_key,
    get_project_name as get_project_name,
    get_saved_workspace_project_keys as get_saved_workspace_project_keys,
    get_thread_cwd as get_thread_cwd,
    is_codex_projectless_chat_cwd as is_codex_projectless_chat_cwd,
    is_thread_mirrorable as is_thread_mirrorable,
    normalize_project_key as normalize_project_key,
    project_keys_match as project_keys_match,
)
from codex_discord_project_types import (
    BridgeProjectModule as BridgeProjectModule,
    FindProjectlessNewChatCwd as FindProjectlessNewChatCwd,
    GetMirroredCodexThreadId as GetMirroredCodexThreadId,
    GetMirrorProjectForChannel as GetMirrorProjectForChannel,
    GetThreadCwd as GetThreadCwd,
    InitMirrorDb as InitMirrorDb,
    ProjectKeysMatch as ProjectKeysMatch,
    ProjectThread as ProjectThread,
    SqlRow as SqlRow,
)


def resolve_discord_new_thread_cwd(
    discord_channel_id: int | None,
    *,
    bridge_module: BridgeProjectModule,
    projectless_chat_key: str,
    get_mirrored_codex_thread_id_func: GetMirroredCodexThreadId,
    get_thread_cwd_func: GetThreadCwd,
    get_mirror_project_for_channel_func: GetMirrorProjectForChannel,
    find_projectless_new_chat_cwd_func: FindProjectlessNewChatCwd,
) -> str | None:
    target_thread_id = get_mirrored_codex_thread_id_func(discord_channel_id)
    thread_cwd = get_thread_cwd_func(target_thread_id)
    if thread_cwd:
        return thread_cwd

    project = get_mirror_project_for_channel_func(discord_channel_id)
    if not project:
        return None
    project_key, _project_name = project
    if project_key == projectless_chat_key:
        return find_projectless_new_chat_cwd_func()
    if project_key and not project_key.startswith("projectless:"):
        project_path = Path(bridge_module.strip_windows_extended_prefix(project_key))
        if project_path.is_dir():
            return str(project_path)
    return None


def resolve_discord_new_thread_project_channel_id(
    discord_channel_id: int | None,
    project_key: str | None,
    *,
    db_path: Path,
    init_mirror_db_func: InitMirrorDb,
    project_keys_match_func: ProjectKeysMatch,
) -> int | None:
    return project_channels.resolve_discord_new_thread_project_channel_id(
        discord_channel_id,
        project_key,
        db_path=db_path,
        init_mirror_db_func=init_mirror_db_func,
        project_keys_match_func=project_keys_match_func,
    )

from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from codex_thread_models import ThreadInfo


class MirrorScopeBridge(Protocol):
    def load_user_root_threads(self) -> list[ThreadInfo]: ...

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]: ...

    def filter_thread_list_for_target(
        self,
        threads: list[ThreadInfo],
        target_thread_id: str,
        cwd: str | None,
    ) -> list[ThreadInfo]: ...


def bounded_mirror_limit(limit: int) -> int:
    return max(1, min(100, int(limit)))


def load_mirror_scope_threads(
    bridge_module: MirrorScopeBridge,
    limit: int | None = None,
) -> list[ThreadInfo]:
    if limit is None:
        return bridge_module.load_user_root_threads()
    return bridge_module.load_recent_threads(bounded_mirror_limit(limit))


def filter_threads_for_discord_channel(
    threads: list[ThreadInfo],
    channel_id: int | None,
    *,
    bridge_module: MirrorScopeBridge,
    get_mirrored_codex_thread_id: Callable[[int | None], str | None],
    get_mirror_project_for_channel: Callable[[int | None], tuple[str, str] | None],
    project_keys_match: Callable[[str, str], bool],
    get_project_key: Callable[[ThreadInfo], str],
) -> list[ThreadInfo]:
    if channel_id is None:
        return threads
    target_thread_id = get_mirrored_codex_thread_id(channel_id)
    if target_thread_id:
        return bridge_module.filter_thread_list_for_target(threads, target_thread_id, None)
    project = get_mirror_project_for_channel(channel_id)
    if project is None:
        return threads
    project_key, _project_name = project
    return [thread for thread in threads if project_keys_match(get_project_key(thread), project_key)]

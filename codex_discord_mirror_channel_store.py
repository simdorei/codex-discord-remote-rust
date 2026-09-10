from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_store as discord_store
from codex_thread_models import ThreadInfo

FetchFailureTypes = tuple[type[Exception], ...]
GetThreadUiNameFunc = Callable[[str, ThreadInfo], str | None]
LogFunc = Callable[[str], None]
NormalizeProjectKeyFunc = Callable[[str | None], str]
ProjectKeysMatchFunc = Callable[[str | None, str | None], bool]


@dataclass(frozen=True, slots=True)
class MirrorChannelDeps:
    db_path: Path
    normalize_project_key: NormalizeProjectKeyFunc
    project_keys_match: ProjectKeysMatchFunc
    get_thread_ui_name: GetThreadUiNameFunc
    log: LogFunc
    fetch_failure_types: FetchFailureTypes


def upsert_mirror_project(
    project_key: str,
    project_name: str,
    channel_id: int,
    *,
    deps: MirrorChannelDeps,
) -> None:
    canonical_project_key = deps.normalize_project_key(project_key)
    merged_aliases = discord_store.upsert_mirror_project(
        deps.db_path,
        canonical_project_key,
        project_name,
        int(channel_id),
        project_keys_match_func=deps.project_keys_match,
    )
    if merged_aliases:
        deps.log(
            f"mirror_project_aliases_merged project={canonical_project_key[:80]} "
            + f"aliases={len(merged_aliases)}"
        )


def find_mirror_project_row_by_key(
    project_key: str | None,
    *,
    deps: MirrorChannelDeps,
) -> tuple[int, str] | None:
    canonical_project_key = deps.normalize_project_key(project_key)
    return discord_store.find_mirror_project_row_by_key(
        deps.db_path,
        canonical_project_key,
        project_keys_match_func=deps.project_keys_match,
    )


def upsert_mirror_thread(
    codex_thread: ThreadInfo,
    project_key: str,
    thread_name: str,
    project_channel_id: int,
    discord_thread_id: int,
    *,
    deps: MirrorChannelDeps,
) -> None:
    canonical_project_key = deps.normalize_project_key(project_key)
    discord_store.upsert_mirror_thread(
        deps.db_path,
        codex_thread.id,
        canonical_project_key,
        thread_name,
        int(project_channel_id),
        int(discord_thread_id),
    )

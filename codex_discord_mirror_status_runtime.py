from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, cast

import codex_discord_mirror_access as discord_mirror_access
import codex_discord_mirror_scope as discord_mirror_scope
import codex_discord_mirror_status as discord_mirror_status
from codex_thread_models import ThreadInfo


class MirrorStatusBridge(
    discord_mirror_status.MirrorListBridge,
    discord_mirror_status.MirrorCheckBridge,
    Protocol,
):
    pass


@dataclass(frozen=True, slots=True)
class MirrorStatusRuntimeDeps:
    db_path: Path
    init_mirror_db: Callable[[], None]
    get_mirror_status_bridge_module: Callable[[], MirrorStatusBridge]
    load_mirror_scope_threads: Callable[[int | None], list[ThreadInfo]]
    filter_threads_for_discord_channel: Callable[[list[ThreadInfo], int | None], list[ThreadInfo]]
    filter_mirrorable_threads: Callable[[list[ThreadInfo]], list[ThreadInfo]]
    filter_app_server_available_threads: Callable[[list[ThreadInfo]], list[ThreadInfo]]
    get_project_key: Callable[[ThreadInfo], str]
    get_project_name: Callable[[ThreadInfo], str]


def resolve_mirror_list_scope(
    limit: int | None = None,
    *,
    channel_id: int | None = None,
    deps: MirrorStatusRuntimeDeps,
) -> tuple[int, list[str] | None]:
    scoped_thread_ids = None
    if limit is None or channel_id is not None:
        scoped_threads = deps.load_mirror_scope_threads(limit)
        scoped_threads = deps.filter_threads_for_discord_channel(scoped_threads, channel_id)
        resolved_limit = len(scoped_threads)
        scoped_thread_ids = [thread.id for thread in scoped_threads]
    else:
        resolved_limit = discord_mirror_scope.bounded_mirror_limit(limit)
    return resolved_limit, scoped_thread_ids


async def inspect_mirror_access_statuses(
    bot: discord_mirror_access.MirrorAccessBot,
    targets: Sequence[discord_mirror_access.ThreadAccessTarget],
) -> dict[int, discord_mirror_access.MirrorThreadAccessStatus]:
    return await discord_mirror_access.inspect_thread_access_map(bot, targets)


def build_mirror_list(
    limit: int | None = None,
    *,
    channel_id: int | None = None,
    deps: MirrorStatusRuntimeDeps,
) -> str:
    resolved_limit, scoped_thread_ids = resolve_mirror_list_scope(
        limit,
        channel_id=channel_id,
        deps=deps,
    )
    return discord_mirror_status.build_mirror_list(
        resolved_limit,
        scoped_thread_ids=scoped_thread_ids,
        db_path=deps.db_path,
        init_mirror_db_func=deps.init_mirror_db,
        bridge_module=deps.get_mirror_status_bridge_module(),
    )


async def build_mirror_list_for_prefix(
    bot: discord_mirror_access.MirrorAccessBot,
    limit: int | None = None,
    *,
    channel_id: int | None = None,
    deps: MirrorStatusRuntimeDeps,
) -> str:
    resolved_limit, scoped_thread_ids = resolve_mirror_list_scope(
        limit,
        channel_id=channel_id,
        deps=deps,
    )
    targets = await asyncio.to_thread(
        discord_mirror_status.load_mirror_list_access_targets,
        resolved_limit,
        scoped_thread_ids=scoped_thread_ids,
        db_path=deps.db_path,
        init_mirror_db_func=deps.init_mirror_db,
    )
    access_statuses = await inspect_mirror_access_statuses(
        bot,
        cast(Sequence[discord_mirror_access.ThreadAccessTarget], targets),
    )
    return await asyncio.to_thread(
        discord_mirror_status.build_mirror_list,
        resolved_limit,
        scoped_thread_ids=scoped_thread_ids,
        db_path=deps.db_path,
        init_mirror_db_func=deps.init_mirror_db,
        bridge_module=deps.get_mirror_status_bridge_module(),
        access_statuses=access_statuses,
    )


def build_mirror_check(
    limit: int | None = None,
    *,
    channel_id: int | None = None,
    access_statuses: discord_mirror_status.MirrorAccessStatusMap | None = None,
    deps: MirrorStatusRuntimeDeps,
) -> str:
    threads = deps.load_mirror_scope_threads(limit)
    threads = deps.filter_threads_for_discord_channel(threads, channel_id)
    available_threads = deps.filter_app_server_available_threads(threads)
    app_server_unavailable_count = len(threads) - len(available_threads)
    bridge_module = deps.get_mirror_status_bridge_module()
    archive_recommended_count = sum(
        1
        for thread in threads
        if bridge_module.should_recommend_archive(
            thread,
            bridge_module.get_thread_context_usage(thread),
        )
    )
    return discord_mirror_status.build_mirror_check(
        threads=threads,
        db_path=deps.db_path,
        init_mirror_db_func=deps.init_mirror_db,
        bridge_module=bridge_module,
        filter_mirrorable_threads_func=lambda items: items,
        get_project_key_func=deps.get_project_key,
        get_project_name_func=deps.get_project_name,
        archive_recommended_count=archive_recommended_count,
        app_server_unavailable_count=app_server_unavailable_count,
        scoped_project_keys=(
            {deps.get_project_key(thread) for thread in threads}
            if channel_id is not None
            else None
        ),
        access_statuses=access_statuses,
    )


async def build_mirror_check_for_prefix(
    bot: discord_mirror_access.MirrorAccessBot,
    limit: int | None = None,
    *,
    channel_id: int | None = None,
    deps: MirrorStatusRuntimeDeps,
) -> str:
    _ = channel_id
    targets = await asyncio.to_thread(
        discord_mirror_status.load_mirror_check_access_targets,
        db_path=deps.db_path,
        init_mirror_db_func=deps.init_mirror_db,
        scoped_project_keys=None,
    )
    access_statuses = await inspect_mirror_access_statuses(
        bot,
        cast(Sequence[discord_mirror_access.ThreadAccessTarget], targets),
    )
    return await asyncio.to_thread(
        build_mirror_check,
        limit,
        channel_id=None,
        access_statuses=access_statuses,
        deps=deps,
    )

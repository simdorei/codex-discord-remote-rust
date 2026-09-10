from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import time
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, TypeVar
from typing import Protocol, cast

import discord

import codex_discord_mirror_channels as discord_mirror_channels
import codex_discord_mirror_coordination as discord_mirror_coordination
import codex_discord_mirror_orphans as discord_mirror_orphans
import codex_discord_mirror_scope as discord_mirror_scope
import codex_discord_mirror_stale as discord_mirror_stale
import codex_discord_mirror_sync_result as discord_mirror_sync_result
import codex_discord_store as discord_store
from codex_discord_id_values import coerce_discord_id_value

BotT = TypeVar("BotT")
GuildT = TypeVar("GuildT")
CategoryT = TypeVar("CategoryT")
ProjectChannelT = TypeVar("ProjectChannelT")
ThreadChannelT = TypeVar("ThreadChannelT")

DISCORD_FETCH_FAILURE_EXCEPTIONS: tuple[type[Exception], ...] = (
    discord.DiscordException,
    OSError,
    RuntimeError,
)


class MirrorCleanupThread(Protocol):
    @property
    def id(self) -> str: ...


CleanupThreadT = TypeVar("CleanupThreadT", bound=MirrorCleanupThread)
ThreadT = TypeVar("ThreadT", bound=MirrorCleanupThread)


@dataclass(frozen=True, slots=True)
class MirrorThreadEnsureDeps(Generic[ThreadT, GuildT, CategoryT, ProjectChannelT, ThreadChannelT]):
    get_project_key: Callable[[ThreadT], str]
    get_project_name: Callable[[ThreadT], str]
    get_or_create_project_channel: Callable[[GuildT, CategoryT, str, str], Awaitable[ProjectChannelT]]
    get_or_create_thread_channel: Callable[[ThreadT, str, ProjectChannelT], Awaitable[ThreadChannelT]]


@dataclass(frozen=True, slots=True)
class MirrorThreadEnsureResult(Generic[ProjectChannelT]):
    projects: dict[str, ProjectChannelT]
    mirrored: int


@dataclass(frozen=True, slots=True)
class MirrorFullCleanupResult:
    stale_threads: list[discord_mirror_stale.StaleMirrorThreadRow]
    stale_projects: list[discord_mirror_stale.StaleMirrorProjectRow]
    stale_cleanup: discord_mirror_sync_result.MirrorCleanupResult
    stale_project_cleanup: discord_mirror_sync_result.MirrorCleanupResult
    orphan_cleanup: discord_mirror_sync_result.MirrorCleanupResult


@dataclass(frozen=True, slots=True)
class CodexMirrorSyncDeps(Generic[BotT, ThreadT, GuildT, CategoryT, ProjectChannelT, ThreadChannelT]):
    db_path: Path
    get_mirror_guild: Callable[[BotT], Awaitable[GuildT]]
    get_or_create_mirror_category: Callable[[GuildT], Awaitable[CategoryT]]
    load_mirror_scope_threads: Callable[[int | None], list[ThreadT]]
    filter_mirrorable_threads: Callable[[list[ThreadT]], list[ThreadT]]
    filter_app_server_available_threads: Callable[[list[ThreadT]], list[ThreadT]]
    get_project_key: Callable[[ThreadT], str]
    get_project_name: Callable[[ThreadT], str]
    get_or_create_project_channel: Callable[[GuildT, CategoryT, str, str], Awaitable[ProjectChannelT]]
    get_or_create_thread_channel: Callable[[ThreadT, str, ProjectChannelT], Awaitable[ThreadChannelT]]
    get_bot_user_id: Callable[[BotT], int | None]
    log: Callable[[str], None]


def _stale_thread_rows_for_discord_delete(
    stale_rows: Sequence[discord_mirror_stale.StaleMirrorThreadRow],
    retained_thread_ids: set[int],
) -> list[discord_mirror_stale.StaleMirrorThreadRow]:
    return [
        row
        for row in stale_rows
        if (thread_id := coerce_discord_id_value(row[1])) is None or thread_id not in retained_thread_ids
    ]


def _stale_project_rows_for_discord_delete(
    stale_rows: Sequence[discord_mirror_stale.StaleMirrorProjectRow],
    retained_project_channel_ids: list[int],
) -> list[discord_mirror_stale.StaleMirrorProjectRow]:
    retained_ids = set(retained_project_channel_ids)
    return [
        row
        for row in stale_rows
        if (channel_id := coerce_discord_id_value(row[2])) is None or channel_id not in retained_ids
    ]


async def ensure_mirror_threads(
    guild: GuildT,
    category: CategoryT,
    threads: Sequence[ThreadT],
    *,
    deps: MirrorThreadEnsureDeps[ThreadT, GuildT, CategoryT, ProjectChannelT, ThreadChannelT],
) -> MirrorThreadEnsureResult[ProjectChannelT]:
    projects: dict[str, ProjectChannelT] = {}
    mirrored = 0
    for codex_thread in reversed(threads):
        project_key = deps.get_project_key(codex_thread)
        project_name = deps.get_project_name(codex_thread)
        channel = projects.get(project_key)
        if channel is None:
            channel = await deps.get_or_create_project_channel(
                guild,
                category,
                project_key,
                project_name,
            )
            projects[project_key] = channel
        _ = await deps.get_or_create_thread_channel(codex_thread, project_key, channel)
        mirrored += 1
    return MirrorThreadEnsureResult(projects=projects, mirrored=mirrored)


@discord_mirror_coordination.serialize_mirror_mutation
async def sync_codex_mirror(
    bot: BotT,
    *,
    limit: int | None = None,
    deps: CodexMirrorSyncDeps[BotT, ThreadT, GuildT, CategoryT, ProjectChannelT, ThreadChannelT],
) -> str:
    cleanup_updated_before = time.time()
    scope = "db-root" if limit is None else str(discord_mirror_scope.bounded_mirror_limit(limit))
    cleanup_scope = "full_db_root" if limit is None else "limited_sync_no_prune"
    deps.log(f"mirror_sync_start limit={scope}")
    guild = await deps.get_mirror_guild(bot)
    category = await deps.get_or_create_mirror_category(guild)
    scope_threads = await asyncio.to_thread(deps.load_mirror_scope_threads, limit)
    threads = await asyncio.to_thread(deps.filter_app_server_available_threads, scope_threads)
    app_server_unavailable_count = len(scope_threads) - len(threads)
    if app_server_unavailable_count:
        deps.log(f"mirror_sync_app_server_unavailable count={app_server_unavailable_count}")

    mirror_result = await ensure_mirror_threads(
        guild,
        category,
        threads,
        deps=MirrorThreadEnsureDeps(
            deps.get_project_key,
            deps.get_project_name,
            deps.get_or_create_project_channel,
            deps.get_or_create_thread_channel,
        ),
    )

    if limit is None:
        cleanup_result = await cleanup_full_mirror_sync(
            cast(discord.Guild, guild),
            cast(discord.CategoryChannel, category),
            scope_threads,
            bot_user_id=deps.get_bot_user_id(bot),
            db_path=deps.db_path,
            get_project_key=deps.get_project_key,
            updated_before=cleanup_updated_before,
        )
        stale_threads = cleanup_result.stale_threads
        stale_projects = cleanup_result.stale_projects
        stale_cleanup = cleanup_result.stale_cleanup
        stale_project_cleanup = cleanup_result.stale_project_cleanup
        orphan_cleanup = cleanup_result.orphan_cleanup
    else:
        stale_threads = []
        stale_projects = []
        stale_cleanup = discord_mirror_sync_result.empty_mirror_cleanup_result()
        stale_project_cleanup = discord_mirror_sync_result.empty_mirror_cleanup_result()
        orphan_cleanup = discord_mirror_sync_result.empty_mirror_cleanup_result()
        deps.log(f"mirror_sync_cleanup_skipped scope={cleanup_scope} limit={scope}")

    deps.log(
        "mirror_sync_done "
        + f"mirrored={mirror_result.mirrored} cleanup_scope={cleanup_scope} stale_rows={len(stale_threads)} "
        + f"stale_deleted={discord_mirror_sync_result.cleanup_count(stale_cleanup, 'deleted')} "
        + f"orphan_deleted={discord_mirror_sync_result.cleanup_count(orphan_cleanup, 'deleted')} "
        + f"orphan_failed={discord_mirror_sync_result.cleanup_count(orphan_cleanup, 'failed')} "
        + f"stale_projects_deleted={discord_mirror_sync_result.cleanup_count(stale_project_cleanup, 'deleted')}"
    )

    return discord_mirror_sync_result.format_mirror_sync_result(
        cleanup_scope=cleanup_scope,
        project_count=len(mirror_result.projects),
        mirrored=mirror_result.mirrored,
        stale_thread_count=len(stale_threads),
        stale_project_count=len(stale_projects),
        stale_cleanup=stale_cleanup,
        orphan_cleanup=orphan_cleanup,
        stale_project_cleanup=stale_project_cleanup,
        db_path=deps.db_path,
        app_server_unavailable_count=app_server_unavailable_count,
    )


async def cleanup_full_mirror_sync(
    guild: discord.Guild,
    category: discord.CategoryChannel,
    threads: Sequence[CleanupThreadT],
    *,
    bot_user_id: int | None,
    db_path: Path,
    get_project_key: Callable[[CleanupThreadT], str],
    updated_before: float,
) -> MirrorFullCleanupResult:
    valid_thread_ids = {thread.id for thread in threads}
    valid_project_keys = {get_project_key(thread) for thread in threads}
    stale_threads = cast(
        list[discord_mirror_stale.StaleMirrorThreadRow],
        discord_store.get_stale_mirror_thread_rows(db_path, valid_thread_ids, updated_before=updated_before),
    )
    stale_projects = cast(
        list[discord_mirror_stale.StaleMirrorProjectRow],
        discord_store.get_stale_mirror_project_rows(db_path, valid_project_keys, updated_before=updated_before),
    )

    discord_store.delete_stale_mirror_rows(
        db_path, valid_thread_ids, valid_project_keys, updated_before=updated_before
    )
    known_thread_ids, project_channel_ids = discord_store.get_remaining_mirror_discord_ids(db_path)
    stale_cleanup = await discord_mirror_stale.delete_stale_discord_threads(
        guild,
        _stale_thread_rows_for_discord_delete(stale_threads, known_thread_ids),
    )
    stale_project_cleanup = await discord_mirror_stale.delete_stale_project_channels(
        guild,
        category,
        _stale_project_rows_for_discord_delete(stale_projects, project_channel_ids),
    )

    project_channels = await discord_mirror_channels.resolve_orphan_cleanup_project_channels(
        guild,
        project_channel_ids,
        fetch_failure_types=DISCORD_FETCH_FAILURE_EXCEPTIONS,
    )
    orphan_cleanup = await discord_mirror_orphans.cleanup_orphan_discord_threads(
        project_channels,
        known_thread_ids,
        bot_user_id,
        is_known_thread_id=lambda thread_id: discord_store.is_mirrored_channel_id(
            db_path, thread_id
        ),
        delivery_exceptions=(discord.Forbidden, discord.HTTPException),
    )
    return MirrorFullCleanupResult(
        stale_threads=stale_threads,
        stale_projects=stale_projects,
        stale_cleanup=stale_cleanup,
        stale_project_cleanup=stale_project_cleanup,
        orphan_cleanup=orphan_cleanup,
    )

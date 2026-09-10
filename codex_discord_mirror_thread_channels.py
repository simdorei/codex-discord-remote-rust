from __future__ import annotations

import discord

import codex_discord_mirror_names as discord_mirror_names
import codex_discord_mirror_thread_store as mirror_thread_store
import codex_discord_store as discord_store
from codex_discord_mirror_channel_store import MirrorChannelDeps, upsert_mirror_thread
from codex_discord_mirror_thread_reuse import find_existing_thread_channel
from codex_thread_models import ThreadInfo


def get_mirror_thread_name(
    codex_thread: ThreadInfo,
    *,
    deps: MirrorChannelDeps,
) -> str:
    def get_thread_ui_name(thread_id: str, thread: ThreadInfo) -> str:
        return deps.get_thread_ui_name(thread_id, thread) or ""

    return discord_mirror_names.get_mirror_thread_name(
        codex_thread,
        get_thread_ui_name=get_thread_ui_name,
    )


async def ensure_mirror_thread_channel(
    codex_thread: ThreadInfo,
    project_key: str,
    project_channel: discord.TextChannel,
    discord_thread: discord.Thread,
    thread_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread:
    if str(getattr(discord_thread, "name", "") or "") != thread_name:
        _ = await discord_thread.edit(name=thread_name, reason="Codex thread mirror sync")
        deps.log(
            f"mirror_thread_renamed codex_thread={codex_thread.id} "
            + f"discord_thread={discord_thread.id} name={thread_name[:80]}"
        )
    upsert_mirror_thread(
        codex_thread,
        project_key,
        thread_name,
        int(project_channel.id),
        int(discord_thread.id),
        deps=deps,
    )
    return discord_thread


async def _ensure_stored_mirror_thread_channel(
    codex_thread: ThreadInfo,
    project_key: str,
    project_channel: discord.TextChannel,
    thread_name: str,
    row: tuple[int, int],
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread:
    thread_id = int(row[1])
    try:
        _, thread_id = mirror_thread_store.stored_mirror_thread_ids(
            row,
            codex_thread,
            project_channel,
        )
        discord_thread = await mirror_thread_store.fetch_stored_discord_thread(
            codex_thread,
            project_channel,
            thread_id,
            deps=deps,
        )
    except RuntimeError as exc:
        compact_reason = " ".join(str(exc).split())[:160]
        if not _is_stale_mirror_thread_error(compact_reason):
            raise
        existing_thread = await _reuse_existing_mirror_thread_channel(
            codex_thread,
            project_key,
            project_channel,
            thread_name,
            deps=deps,
        )
        remapped_thread = existing_thread
        if remapped_thread is None:
            remapped_thread = await _create_mirror_thread_channel(
                codex_thread,
                project_key,
                project_channel,
                thread_name,
                deps=deps,
            )
        deps.log(
            f"mirror_thread_remapped codex_thread={codex_thread.id} "
            + f"old_id={thread_id} new_id={remapped_thread.id} reason={compact_reason}"
        )
        return remapped_thread
    return await ensure_mirror_thread_channel(
        codex_thread,
        project_key,
        project_channel,
        discord_thread,
        thread_name,
        deps=deps,
    )


def _is_stale_mirror_thread_error(message: str) -> bool:
    stale_markers = (
        "Unknown Channel",
        "Forbidden",
        "NotFound",
        "not found",
        "not Thread",
        "not current project channel",
        "not project channel",
        "404",
        "403",
    )
    return any(marker in message for marker in stale_markers)


async def _reuse_existing_mirror_thread_channel(
    codex_thread: ThreadInfo,
    project_key: str,
    project_channel: discord.TextChannel,
    thread_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread | None:
    existing_thread = await find_existing_thread_channel(project_channel, thread_name, deps=deps)
    if existing_thread is None:
        return None
    existing_owner = discord_store.get_mirrored_codex_thread_id(deps.db_path, int(existing_thread.id))
    if existing_owner is not None and existing_owner != codex_thread.id:
        deps.log(
            f"mirror_thread_reuse_skipped codex_thread={codex_thread.id} "
            + f"discord_thread={existing_thread.id} existing_codex_thread={existing_owner}"
        )
        return None
    ensured_thread = await ensure_mirror_thread_channel(
        codex_thread,
        project_key,
        project_channel,
        existing_thread,
        thread_name,
        deps=deps,
    )
    deps.log(
        f"mirror_thread_reused codex_thread={codex_thread.id} "
        + f"discord_thread={ensured_thread.id}"
    )
    return ensured_thread


async def _create_mirror_thread_channel(
    codex_thread: ThreadInfo,
    project_key: str,
    project_channel: discord.TextChannel,
    thread_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread:
    discord_thread = await project_channel.create_thread(
        name=thread_name,
        type=discord.ChannelType.public_thread,
        auto_archive_duration=10080,
        reason="Codex thread mirror sync",
    )
    upsert_mirror_thread(
        codex_thread,
        project_key,
        thread_name,
        int(project_channel.id),
        int(discord_thread.id),
        deps=deps,
    )
    return discord_thread


async def get_or_create_thread_channel(
    codex_thread: ThreadInfo,
    project_key: str,
    project_channel: discord.TextChannel,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread:
    thread_name = get_mirror_thread_name(codex_thread, deps=deps)
    row = discord_store.get_mirror_thread_row_by_codex_thread_id(
        deps.db_path,
        codex_thread.id,
    )

    if row:
        return await _ensure_stored_mirror_thread_channel(
            codex_thread,
            project_key,
            project_channel,
            thread_name,
            row,
            deps=deps,
        )

    existing_thread = await _reuse_existing_mirror_thread_channel(
        codex_thread,
        project_key,
        project_channel,
        thread_name,
        deps=deps,
    )
    if existing_thread is not None:
        return existing_thread
    return await _create_mirror_thread_channel(
        codex_thread,
        project_key,
        project_channel,
        thread_name,
        deps=deps,
    )

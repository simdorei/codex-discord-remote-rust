from __future__ import annotations

import discord

from codex_discord_mirror_channel_store import MirrorChannelDeps
from codex_thread_models import ThreadInfo


def _require_project_thread(
    candidate: discord.abc.GuildChannel | discord.Thread | None,
    codex_thread: ThreadInfo,
    project_channel: discord.TextChannel,
) -> discord.Thread:
    thread_id = int(getattr(candidate, "id", 0) or 0)
    if not isinstance(candidate, discord.Thread):
        raise RuntimeError(
            f"Stored mirror thread {thread_id} for {codex_thread.id} "
            + f"is {type(candidate).__name__}, not Thread."
        )
    parent_id = int(candidate.parent_id or 0)
    if parent_id != int(project_channel.id):
        raise RuntimeError(
            f"Stored mirror thread {thread_id} for {codex_thread.id} belongs to "
            + f"Discord channel {parent_id}, not project channel {project_channel.id}."
        )
    return candidate


def stored_mirror_thread_ids(
    row: tuple[int, int],
    codex_thread: ThreadInfo,
    project_channel: discord.TextChannel,
) -> tuple[int, int]:
    channel_id = int(row[0])
    thread_id = int(row[1])
    if channel_id != int(project_channel.id):
        raise RuntimeError(
            f"Stored mirror thread {thread_id} for {codex_thread.id} belongs to "
            + f"project channel {channel_id}, not current project channel {project_channel.id}."
        )
    return channel_id, thread_id


async def fetch_stored_discord_thread(
    codex_thread: ThreadInfo,
    project_channel: discord.TextChannel,
    thread_id: int,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread:
    cached = project_channel.guild.get_thread(thread_id)
    if cached is not None:
        return _require_project_thread(cached, codex_thread, project_channel)
    try:
        fetched = await project_channel.guild.fetch_channel(thread_id)
    except deps.fetch_failure_types as exc:
        raise RuntimeError(
            f"Stored mirror thread {thread_id} for {codex_thread.id} "
            + f"is unavailable: {type(exc).__name__}: {exc}"
        ) from exc
    return _require_project_thread(fetched, codex_thread, project_channel)

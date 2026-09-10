from __future__ import annotations

from collections.abc import Iterable

import discord

from codex_discord_mirror_names import get_mirror_project_channel_name
from codex_discord_mirror_project_candidates import find_existing_project_channel
from codex_discord_mirror_channel_store import (
    FetchFailureTypes,
    MirrorChannelDeps,
    find_mirror_project_row_by_key,
    upsert_mirror_project,
)
from codex_discord_text import normalize_discord_name


async def resolve_orphan_cleanup_project_channels(
    guild: discord.Guild,
    project_channel_ids: Iterable[int],
    *,
    fetch_failure_types: FetchFailureTypes,
) -> list[discord.TextChannel]:
    channels: list[discord.TextChannel] = []
    for channel_id in project_channel_ids:
        channel = guild.get_channel(channel_id)
        if channel is None:
            try:
                channel = await guild.fetch_channel(channel_id)
            except fetch_failure_types:
                channel = None
        if isinstance(channel, discord.TextChannel):
            channels.append(channel)
    return channels


async def ensure_mirror_project_channel(
    guild: discord.Guild,
    channel: discord.TextChannel,
    project_key: str,
    project_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel:
    expected_name = get_mirror_project_channel_name(
        guild,
        project_key,
        project_name,
        current_channel_id=int(channel.id),
    )
    expected_topic = f"Codex project mirror: {project_name}"
    updates: dict[str, str] = {}
    if str(getattr(channel, "name", "") or "") != expected_name:
        updates["name"] = expected_name
    if str(getattr(channel, "topic", "") or "") != expected_topic:
        updates["topic"] = expected_topic
    if updates:
        if "name" in updates and "topic" in updates:
            _ = await channel.edit(
                name=updates["name"],
                topic=updates["topic"],
                reason="Codex project mirror sync",
            )
        elif "name" in updates:
            _ = await channel.edit(
                name=updates["name"],
                reason="Codex project mirror sync",
            )
        else:
            _ = await channel.edit(
                topic=updates["topic"],
                reason="Codex project mirror sync",
            )
        deps.log(
            f"mirror_project_renamed project={project_key[:80]} "
            + f"channel={channel.id} name={expected_name}"
        )
    upsert_mirror_project(project_key, project_name, int(channel.id), deps=deps)
    return channel


async def _fetch_stored_project_channel(
    guild: discord.Guild,
    stored_channel_id: int,
    project_key: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel | None:
    channel = guild.get_channel(stored_channel_id)
    if isinstance(channel, discord.TextChannel):
        return channel
    try:
        fetched = await guild.fetch_channel(stored_channel_id)
    except deps.fetch_failure_types as exc:
        if isinstance(exc, discord.NotFound):
            return None
        raise RuntimeError(
            f"Stored mirror project channel {stored_channel_id} for {project_key} "
            + f"is unavailable: {type(exc).__name__}: {exc}"
        ) from exc
    if not isinstance(fetched, discord.TextChannel):
        raise RuntimeError(
            f"Stored mirror project channel {stored_channel_id} for {project_key} "
            + f"is {type(fetched).__name__}, not TextChannel."
        )
    return fetched


async def _ensure_stored_project_channel(
    guild: discord.Guild,
    project_key: str,
    project_name: str,
    row: tuple[int, str],
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel | None:
    stored_channel_id = int(row[0])
    channel = await _fetch_stored_project_channel(
        guild,
        stored_channel_id,
        project_key,
        deps=deps,
    )
    if channel is None:
        return None
    return await ensure_mirror_project_channel(
        guild,
        channel,
        project_key,
        project_name,
        deps=deps,
    )


async def _reuse_existing_project_channel(
    guild: discord.Guild,
    project_key: str,
    project_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel | None:
    base_name = normalize_discord_name(project_name, prefix="codex-", max_len=80)
    existing_channel = find_existing_project_channel(
        guild,
        project_name=project_name,
        base_name=base_name,
    )
    if existing_channel is None:
        return None
    ensured_channel = await ensure_mirror_project_channel(
        guild,
        existing_channel,
        project_key,
        project_name,
        deps=deps,
    )
    deps.log(
        f"mirror_project_reused project={project_key[:80]} "
        + f"channel={ensured_channel.id}"
    )
    return ensured_channel


async def _create_project_channel(
    guild: discord.Guild,
    category: discord.CategoryChannel,
    project_key: str,
    project_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel:
    channel_name = get_mirror_project_channel_name(guild, project_key, project_name)
    channel = await guild.create_text_channel(
        channel_name,
        category=category,
        topic=f"Codex project mirror: {project_name}",
        reason="Codex project mirror sync",
    )
    upsert_mirror_project(project_key, project_name, int(channel.id), deps=deps)
    return channel


async def get_or_create_project_channel(
    guild: discord.Guild,
    category: discord.CategoryChannel,
    project_key: str,
    project_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.TextChannel:
    project_key = deps.normalize_project_key(project_key)
    row = find_mirror_project_row_by_key(project_key, deps=deps)

    if row:
        stored_channel = await _ensure_stored_project_channel(
            guild,
            project_key,
            project_name,
            row,
            deps=deps,
        )
        if stored_channel is not None:
            return stored_channel
        deps.log(
            f"mirror_project_stored_channel_missing project={project_key[:80]} "
            + f"channel={int(row[0])} action=replace"
        )

    existing_channel = await _reuse_existing_project_channel(
        guild,
        project_key,
        project_name,
        deps=deps,
    )
    if existing_channel is not None:
        return existing_channel
    return await _create_project_channel(
        guild,
        category,
        project_key,
        project_name,
        deps=deps,
    )

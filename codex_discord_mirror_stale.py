from __future__ import annotations

from collections.abc import Sequence
from typing import Protocol, TypeAlias

import discord

import codex_discord_delivery_state as discord_delivery_state
import codex_discord_mirror_sync_result as mirror_sync_result
from codex_discord_id_values import coerce_discord_id_value

StaleMirrorThreadRow: TypeAlias = tuple[str, discord_delivery_state.DiscordIdValue, str | None]
StaleMirrorProjectRow: TypeAlias = tuple[str, str | None, discord_delivery_state.DiscordIdValue]


class ProjectCleanupGuild(Protocol):
    def get_channel(self, channel_id: int, /) -> object | None: ...

    async def fetch_channel(self, channel_id: int, /) -> object: ...


class ProjectCleanupCategory(Protocol):
    @property
    def id(self) -> int | str: ...


async def delete_stale_discord_threads(
    guild: discord.Guild,
    stale_rows: Sequence[StaleMirrorThreadRow],
) -> mirror_sync_result.MirrorCleanupResult:
    deleted = 0
    missing = 0
    failed = 0
    errors: list[str] = []

    for codex_thread_id, discord_thread_id, thread_title in stale_rows:
        thread_id = coerce_discord_id_value(discord_thread_id)
        if thread_id is None:
            missing += 1
            continue

        try:
            channel = guild.get_thread(thread_id)
            if channel is None:
                fetched = await guild.fetch_channel(thread_id)
                channel = fetched if isinstance(fetched, discord.Thread) else None
            if channel is None:
                missing += 1
                continue
            await channel.delete(
                reason=f"Codex mirror cleanup for stale thread {str(codex_thread_id)[:8]}"
            )
            deleted += 1
        except discord.NotFound:
            missing += 1
        except (discord.Forbidden, discord.HTTPException) as exc:
            failed += 1
            if len(errors) < 3:
                label = str(thread_title or codex_thread_id or thread_id)[:80]
                errors.append(f"{label}: {exc}")

    return {
        "deleted": deleted,
        "missing": missing,
        "failed": failed,
        "errors": errors,
    }


async def delete_stale_project_channels(
    guild: ProjectCleanupGuild,
    category: ProjectCleanupCategory,
    stale_rows: Sequence[StaleMirrorProjectRow],
) -> mirror_sync_result.MirrorCleanupResult:
    deleted = 0
    missing = 0
    skipped = 0
    failed = 0
    errors: list[str] = []

    for project_key, project_name, discord_channel_id in stale_rows:
        channel_id = coerce_discord_id_value(discord_channel_id)
        if channel_id is None:
            missing += 1
            continue

        try:
            channel = guild.get_channel(channel_id)
            if channel is None:
                fetched = await guild.fetch_channel(channel_id)
                channel = fetched if isinstance(fetched, discord.TextChannel) else None
            if channel is None:
                missing += 1
                continue
            if not isinstance(channel, discord.TextChannel):
                skipped += 1
                continue

            topic = getattr(channel, "topic", "") or ""
            parent_id = getattr(channel, "category_id", None)
            is_mirror_channel = parent_id == int(category.id) or topic.startswith(
                "Codex project mirror:"
            )
            if not is_mirror_channel:
                skipped += 1
                continue

            await channel.delete(
                reason=f"Codex mirror cleanup for stale project {str(project_key)[:80]}"
            )
            deleted += 1
        except discord.NotFound:
            missing += 1
        except (discord.Forbidden, discord.HTTPException) as exc:
            failed += 1
            if len(errors) < 3:
                label = str(project_name or project_key or channel_id)[:80]
                errors.append(f"{label}: {exc}")

    return {
        "deleted": deleted,
        "missing": missing,
        "skipped": skipped,
        "failed": failed,
        "errors": errors,
    }

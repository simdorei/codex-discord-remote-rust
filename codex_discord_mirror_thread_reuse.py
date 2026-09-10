from __future__ import annotations

import asyncio  # noqa: ANYIO_OK

import discord

from codex_discord_mirror_channel_store import MirrorChannelDeps


async def find_existing_thread_channel(
    project_channel: discord.TextChannel,
    thread_name: str,
    *,
    deps: MirrorChannelDeps,
) -> discord.Thread | None:
    current_threads = list[object](getattr(project_channel, "threads", []))
    for thread in current_threads:
        if isinstance(thread, discord.Thread) and str(getattr(thread, "name", "") or "") == thread_name:
            return thread

    try:
        archive_iter = project_channel.archived_threads(limit=100)
    except (AttributeError, TypeError):
        return None
    except deps.fetch_failure_types as exc:
        deps.log(
            f"mirror_thread_reuse_scan_failed channel={getattr(project_channel, 'id', '-')} "
            + f"error={str(exc)[:120]}"
        )
        return None
    try:
        async with asyncio.timeout(5):
            async for thread in archive_iter:
                if str(getattr(thread, "name", "") or "") == thread_name:
                    return thread
    except TimeoutError:
        deps.log(f"mirror_thread_reuse_scan_timeout channel={getattr(project_channel, 'id', '-')}")
    except deps.fetch_failure_types as exc:
        deps.log(
            f"mirror_thread_reuse_scan_failed channel={getattr(project_channel, 'id', '-')} "
            + f"error={str(exc)[:120]}"
        )
    return None

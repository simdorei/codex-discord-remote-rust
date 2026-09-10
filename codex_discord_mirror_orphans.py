from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import AsyncIterator, Awaitable, Callable, Iterable
from typing import Protocol

import codex_discord_mirror_sync_result as mirror_sync_result


class OrphanMirrorThread(Protocol):
    @property
    def id(self) -> int | str: ...

    @property
    def owner_id(self) -> int | None: ...

    @property
    def name(self) -> str: ...

    def delete(self, *, reason: str) -> Awaitable[None]: ...


class OrphanMirrorProjectChannel(Protocol):
    @property
    def threads(self) -> Iterable[OrphanMirrorThread]: ...

    @property
    def name(self) -> str: ...

    def archived_threads(self, *, limit: int) -> AsyncIterator[OrphanMirrorThread]: ...


async def cleanup_orphan_discord_threads(
    project_channels: Iterable[OrphanMirrorProjectChannel],
    known_thread_ids: set[int],
    bot_user_id: int | None,
    *,
    is_known_thread_id: Callable[[int], bool],
    delivery_exceptions: tuple[type[BaseException], ...],
    archived_limit: int = 50,
    archived_timeout_seconds: float = 5.0,
) -> mirror_sync_result.MirrorCleanupResult:
    deleted = 0
    skipped = 0
    failed = 0
    seen_thread_ids: set[int] = set()
    errors: list[str] = []

    async def maybe_delete_thread(thread: OrphanMirrorThread) -> None:
        nonlocal deleted, skipped, failed
        thread_id = int(thread.id)
        if thread_id in seen_thread_ids:
            return
        seen_thread_ids.add(thread_id)
        if thread_id in known_thread_ids:
            skipped += 1
            return
        if bot_user_id is not None and thread.owner_id not in {None, bot_user_id}:
            skipped += 1
            return
        if is_known_thread_id(thread_id):
            skipped += 1
            return
        try:
            await thread.delete(reason="Codex mirror cleanup for orphan Discord thread")
            deleted += 1
        except delivery_exceptions as exc:
            failed += 1
            if len(errors) < 3:
                errors.append(f"{thread.name}: {exc}")

    for channel in project_channels:
        for thread in list(channel.threads):
            await maybe_delete_thread(thread)
        try:
            async with asyncio.timeout(archived_timeout_seconds):
                async for thread in channel.archived_threads(limit=archived_limit):
                    await maybe_delete_thread(thread)
        except TimeoutError:
            failed += 1
            if len(errors) < 3:
                errors.append(f"{channel.name}/archived_threads: timed out")
        except delivery_exceptions as exc:
            failed += 1
            if len(errors) < 3:
                errors.append(f"{channel.name}/archived_threads: {exc}")

    return {
        "deleted": deleted,
        "skipped": skipped,
        "failed": failed,
        "errors": errors,
    }

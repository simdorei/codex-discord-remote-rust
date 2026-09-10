from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Protocol

from codex_app_server_transport_lifecycle import AppServerLifecycleSnapshot
from codex_discord_queue_job_memory import to_memory_queue_job
from codex_discord_runner_queue import QueueJob, QueueJobValue
import codex_discord_store as store


_MAX_RESTORE_ATTEMPTS = 3


class QueueRestoreUnstableError(RuntimeError):
    pass


class QueueRestoreBot(Protocol):
    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[QueueJobValue, str]: ...
    def fetch_channel(self, channel_id: int) -> Awaitable[QueueJobValue]: ...
    def is_allowed_message_channel(self, channel: QueueJobValue) -> bool: ...


class QueueRestoreDeps(Protocol):
    @property
    def get_db_path(self) -> Callable[[], Path]: ...

    @property
    def get_app_server_lifecycle(self) -> Callable[[], AppServerLifecycleSnapshot]: ...

    @property
    def ensure_app_server_ready(self) -> Callable[[], None]: ...

    @property
    def log(self) -> Callable[[str], None]: ...


async def restore_queue_jobs(bot: QueueRestoreBot, deps: QueueRestoreDeps) -> list[QueueJob]:
    last_start_error: Exception | None = None
    for attempt in range(1, _MAX_RESTORE_ATTEMPTS + 1):
        snapshot = deps.get_app_server_lifecycle()
        if not snapshot.healthy or snapshot.generation <= 0:
            deps.log(
                "queue_restore_retry "
                + f"attempt={attempt} reason=app_server_unhealthy "
                + f"generation={snapshot.generation}"
            )
            try:
                await asyncio.to_thread(deps.ensure_app_server_ready)
            except Exception as exc:  # noqa: BLE001 - retry, then surface the exact startup failure.
                last_start_error = exc
                deps.log(
                    "queue_restore_app_server_start_failed "
                    + f"attempt={attempt} error_type={type(exc).__name__} "
                    + f"error={str(exc)[:300]}"
                )
                if attempt < _MAX_RESTORE_ATTEMPTS:
                    await asyncio.sleep(0.05 * attempt)
                    continue
                raise QueueRestoreUnstableError(
                    "Durable queue restore could not start Codex app-server "
                    + f"after {_MAX_RESTORE_ATTEMPTS} attempts: {exc}"
                ) from exc
            snapshot = deps.get_app_server_lifecycle()

        accepted_generation = snapshot.generation if snapshot.healthy else 0
        if accepted_generation <= 0:
            if attempt < _MAX_RESTORE_ATTEMPTS:
                await asyncio.sleep(0.05 * attempt)
                continue
            break

        adoption = await asyncio.to_thread(
            store.adopt_queue_jobs_generation,
            deps.get_db_path(),
            accepted_generation,
        )
        returned_quarantined = tuple(
            record for record in adoption.jobs if store.is_quarantined_queue_job(record)
        )
        records = tuple(
            record for record in adoption.jobs if not store.is_quarantined_queue_job(record)
        )
        skipped = adoption.quarantined_count + len(returned_quarantined)
        if skipped:
            deps.log(f"queue_restore_quarantined skipped={skipped}")
        if adoption.fork_fenced_count:
            deps.log(
                "queue_restore_fork_fenced "
                + f"skipped={adoption.fork_fenced_count}"
            )
        if adoption.starting_held_count:
            deps.log(
                "queue_restore_starting_candidates_held "
                + f"skipped={adoption.starting_held_count}"
            )
        deps.log(
            "queue_restore_adopted "
            + f"attempt={attempt} generation={accepted_generation} "
            + f"jobs={len(records)} rebound={adoption.adopted_count}"
        )
        jobs: list[QueueJob] = []
        for record in records:
            channel, source = bot.get_cached_channel_or_thread(record.channel_id)
            if channel is None:
                try:
                    channel = await bot.fetch_channel(record.channel_id)
                    source = "fetch"
                except (OSError, RuntimeError, TimeoutError) as exc:
                    deps.log(
                        f"queue_restore_channel_failed channel={record.channel_id} "
                        + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
                    )
                    continue
            if not bot.is_allowed_message_channel(channel):
                deps.log(f"queue_restore_channel_denied channel={record.channel_id} source={source}")
                continue
            jobs.append(to_memory_queue_job(record, channel, None))

        current = deps.get_app_server_lifecycle()
        if current.healthy and current.generation == accepted_generation:
            return jobs
        deps.log(
            "queue_restore_generation_changed "
            + f"attempt={attempt} expected={accepted_generation} "
            + f"current={current.generation if current.healthy else 'unhealthy'}"
        )
        if attempt < _MAX_RESTORE_ATTEMPTS:
            await asyncio.sleep(0)

    suffix = f": {last_start_error}" if last_start_error is not None else ""
    raise QueueRestoreUnstableError(
        "Durable queue restore could not observe a stable Codex app-server "
        + f"generation after {_MAX_RESTORE_ATTEMPTS} attempts{suffix}"
    )

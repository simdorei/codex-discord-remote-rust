"""App-server lifecycle guards for durable Discord queue jobs."""

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from pathlib import Path
from typing import NoReturn, Protocol, cast

from codex_app_server_transport_lifecycle import (
    AppServerGenerationMismatch,
    AppServerLifecycleSnapshot,
)
from codex_discord_queue_processor import QueueGenerationExpiredError
from codex_discord_runner_queue import QueueJobValue
import codex_discord_store as store
from codex_discord_store_queue import StoredQueueJob


LifecycleSnapshotGetter = Callable[[], AppServerLifecycleSnapshot]


class QueueGenerationDeps(Protocol):
    @property
    def get_db_path(self) -> Callable[[], Path]: ...

    @property
    def get_app_server_lifecycle(self) -> LifecycleSnapshotGetter: ...

    @property
    def log(self) -> Callable[[str], None]: ...

    @property
    def notify_app_server_work_changed(self) -> Callable[[], None]: ...


async def require_queue_generation(
    deps: QueueGenerationDeps,
    generation: int,
    stage: str,
) -> None:
    snapshot = deps.get_app_server_lifecycle()
    if snapshot.healthy and generation > 0 and snapshot.generation == generation:
        return
    await raise_queue_generation_expired(deps, generation, stage, snapshot)


async def translate_transport_generation_expiry(
    deps: QueueGenerationDeps,
    exc: AppServerGenerationMismatch,
    generation: int,
    stage: str,
    *,
    job_id: str | None = None,
) -> NoReturn:
    _ = job_id
    snapshot = deps.get_app_server_lifecycle()
    deleted = await discard_stale_queue_jobs(deps, snapshot)
    deps.log(
        "queue_transport_generation_expired "
        + f"stage={stage} expected={generation} actual={exc.actual_generation} "
        + f"healthy={exc.healthy} deleted={len(deleted)}"
    )
    raise QueueGenerationExpiredError(
        stage=stage,
        expected_generation=generation,
        current_generation=exc.actual_generation,
        healthy=exc.healthy,
    ) from exc


async def raise_queue_generation_expired(
    deps: QueueGenerationDeps,
    generation: int,
    stage: str,
    snapshot: AppServerLifecycleSnapshot,
) -> NoReturn:
    current_generation = int(snapshot.generation)
    deleted = await discard_stale_queue_jobs(deps, snapshot)
    deps.log(
        "queue_generation_expired "
        + f"stage={stage} expected={generation} "
        + f"current={current_generation if snapshot.healthy else 'unhealthy'} "
        + f"deleted={len(deleted)}"
    )
    raise QueueGenerationExpiredError(
        stage=stage,
        expected_generation=generation,
        current_generation=current_generation,
        healthy=snapshot.healthy,
    )


async def discard_stale_queue_jobs(
    deps: QueueGenerationDeps,
    snapshot: AppServerLifecycleSnapshot,
) -> list[StoredQueueJob]:
    records = await asyncio.to_thread(store.list_queue_jobs, deps.get_db_path())
    current = deps.get_app_server_lifecycle()
    if current.healthy != snapshot.healthy or current.generation != snapshot.generation:
        deps.log(
            "queue_stale_cleanup_deferred "
            + f"expected={snapshot.generation if snapshot.healthy else 'unhealthy'} "
            + f"current={current.generation if current.healthy else 'unhealthy'}"
        )
        return []
    stale_records = [
        record
        for record in records
        if not snapshot.healthy or record.app_server_generation != snapshot.generation
    ]
    discarded = await asyncio.to_thread(
        store.discard_observed_queue_jobs,
        deps.get_db_path(),
        stale_records,
    )
    if discarded:
        deps.notify_app_server_work_changed()
    return discarded


def reject_source_before_accepting_since(
    deps: QueueGenerationDeps,
    source_message: QueueJobValue,
    snapshot: AppServerLifecycleSnapshot,
    *,
    channel_id: int,
) -> None:
    source_created_at = _source_message_created_timestamp(source_message)
    if (
        source_created_at is None
        or snapshot.accepting_since is None
        or source_created_at >= snapshot.accepting_since
    ):
        return
    deps.log(
        "queue_source_predates_generation "
        + f"generation={snapshot.generation} channel={channel_id}"
    )
    raise QueueGenerationExpiredError(
        stage="enqueue_source_admission",
        expected_generation=snapshot.generation,
        current_generation=snapshot.generation,
        healthy=True,
    )


def _source_message_created_timestamp(source_message: QueueJobValue) -> float | None:
    created_at = cast(object, getattr(source_message, "created_at", None))
    if created_at is None:
        return None
    if isinstance(created_at, int | float):
        return float(created_at)
    timestamp = cast(object, getattr(created_at, "timestamp", None))
    if not callable(timestamp):
        raise TypeError("Discord source message has an invalid created_at value.")
    value = cast(Callable[[], object], timestamp)()
    if not isinstance(value, int | float):
        raise TypeError("Discord source message timestamp is not numeric.")
    return float(value)

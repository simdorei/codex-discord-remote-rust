from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import sqlite3
import traceback
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Generic, TypeAlias, TypeVar

ReadyCleanup = Callable[[], int]
AsyncReadyCleanup = Callable[[], Awaitable[None]]
ChannelT = TypeVar("ChannelT")
StartupProbeTargetsGetter: TypeAlias = Callable[[], Sequence[tuple[str, int]]]
CachedChannelGetter: TypeAlias = Callable[[int], tuple[ChannelT | None, str]]
ChannelFetcher: TypeAlias = Callable[[int], Awaitable[ChannelT]]
ChannelPredicate: TypeAlias = Callable[[ChannelT], bool]
StaleBusyChoiceChannelCleanup: TypeAlias = Callable[[ChannelT], Awaitable[int]]


@dataclass(frozen=True, slots=True)
class StaleBusyChoiceCleanupDeps(Generic[ChannelT]):
    get_startup_probe_targets: StartupProbeTargetsGetter
    get_cached_channel_or_thread: CachedChannelGetter[ChannelT]
    fetch_channel: ChannelFetcher[ChannelT]
    delivery_exceptions: tuple[type[BaseException], ...]
    is_messageable: ChannelPredicate[ChannelT]
    cleanup_channel: StaleBusyChoiceChannelCleanup[ChannelT]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class ReadyMaintenanceDeps:
    cleanup_expired_busy_choices: ReadyCleanup
    cleanup_expired_persistent_component_claims: ReadyCleanup
    cleanup_processed_messages: ReadyCleanup
    cleanup_session_mirror_events: ReadyCleanup
    cleanup_stale_busy_choice_components: AsyncReadyCleanup | None
    log: Callable[[str], None]


async def run_ready_cleanup(
    cleanup: ReadyCleanup,
    *,
    deleted_event: str,
    failed_event: str,
    log: Callable[[str], None],
) -> None:
    try:
        deleted_count = await asyncio.to_thread(cleanup)
        if deleted_count:
            log(f"{deleted_event} count={deleted_count}")
    except (OSError, RuntimeError, sqlite3.Error):
        log(f"{failed_event}\n" + traceback.format_exc())


async def run_ready_maintenance(deps: ReadyMaintenanceDeps) -> None:
    await run_ready_cleanup(
        deps.cleanup_expired_busy_choices,
        deleted_event="busy_choice_cleanup_deleted",
        failed_event="busy_choice_cleanup_failed",
        log=deps.log,
    )
    await run_ready_cleanup(
        deps.cleanup_expired_persistent_component_claims,
        deleted_event="persistent_component_claim_cleanup_deleted",
        failed_event="persistent_component_claim_cleanup_failed",
        log=deps.log,
    )
    await run_ready_cleanup(
        deps.cleanup_processed_messages,
        deleted_event="processed_message_cleanup_deleted",
        failed_event="processed_message_cleanup_failed",
        log=deps.log,
    )
    await run_ready_cleanup(
        deps.cleanup_session_mirror_events,
        deleted_event="session_mirror_event_cleanup_deleted",
        failed_event="session_mirror_event_cleanup_failed",
        log=deps.log,
    )
    if deps.cleanup_stale_busy_choice_components is None:
        return
    try:
        await deps.cleanup_stale_busy_choice_components()
    except (OSError, RuntimeError, sqlite3.Error):
        deps.log("stale_busy_choice_component_cleanup_failed\n" + traceback.format_exc())


async def cleanup_stale_busy_choice_components(
    *,
    deps: StaleBusyChoiceCleanupDeps[ChannelT],
) -> None:
    cleared_total = 0
    for label, channel_id in deps.get_startup_probe_targets():
        channel, _source = deps.get_cached_channel_or_thread(channel_id)
        if channel is None:
            try:
                channel = await deps.fetch_channel(channel_id)
            except deps.delivery_exceptions as exc:
                deps.log(
                    f"stale_busy_choice_component_cleanup_skipped label={label} channel={channel_id} reason=fetch_failed error_type={type(exc).__name__}"
                )
                continue
        if not deps.is_messageable(channel):
            continue
        try:
            cleared = await deps.cleanup_channel(channel)
        except deps.delivery_exceptions:
            deps.log(f"stale_busy_choice_component_cleanup_failed label={label} channel={channel_id}\n" + traceback.format_exc())
            continue
        if cleared:
            cleared_total += cleared
            deps.log(f"stale_busy_choice_component_cleanup_deleted label={label} channel={channel_id} count={cleared}")
    if cleared_total:
        deps.log(f"stale_busy_choice_component_cleanup_done count={cleared_total}")

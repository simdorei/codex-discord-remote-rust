from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Generic, TypeAlias, TypeVar, cast


ChannelT = TypeVar("ChannelT")
LogFunc = Callable[[str], None]
CachedChannelGetter: TypeAlias = Callable[[int], tuple[ChannelT | None, str]]
ChannelFetcher: TypeAlias = Callable[[int], Awaitable[ChannelT]]
ChannelPredicate: TypeAlias = Callable[[ChannelT], bool]
ProbeTargetsGetter: TypeAlias = Callable[[set[int], int | None], Sequence[tuple[str, int]]]
ProbeTimeoutGetter: TypeAlias = Callable[[], float]
StartupProbeRunner: TypeAlias = Callable[[str, int], Awaitable[None]]
TracebackFormatter: TypeAlias = Callable[[], str]


@dataclass(frozen=True, slots=True)
class StartupProbeDeps(Generic[ChannelT]):
    get_cached_channel_or_thread: CachedChannelGetter[ChannelT]
    fetch_channel: ChannelFetcher[ChannelT]
    delivery_exceptions: tuple[type[BaseException], ...]
    is_messageable: ChannelPredicate[ChannelT]
    is_allowed_message_channel: ChannelPredicate[ChannelT]
    log: LogFunc


@dataclass(frozen=True, slots=True)
class StartupDiagnosticsDeps:
    allowed_channel_ids: set[int]
    startup_channel_id: int | None
    get_probe_targets: ProbeTargetsGetter
    get_probe_timeout: ProbeTimeoutGetter
    probe_channel_access: StartupProbeRunner
    delivery_exceptions: tuple[type[BaseException], ...]
    format_traceback: TracebackFormatter
    log: LogFunc


async def probe_channel_access(
    label: str,
    channel_id: int,
    *,
    deps: StartupProbeDeps[ChannelT],
) -> None:
    channel, source = deps.get_cached_channel_or_thread(channel_id)
    if channel is None:
        try:
            channel = await deps.fetch_channel(channel_id)
            source = "fetch"
        except deps.delivery_exceptions as exc:
            deps.log(
                f"startup_channel_probe label={label} channel={channel_id} status=failed source=fetch error_type={type(exc).__name__}"
            )
            return

    allowed_message = False
    messageable = deps.is_messageable(channel)
    if messageable:
        try:
            allowed_message = deps.is_allowed_message_channel(channel)
        except deps.delivery_exceptions:
            allowed_message = False
    parent_id = cast(int | str | None, getattr(channel, "parent_id", None))
    parent_text = "-" if parent_id is None else str(parent_id)
    deps.log(
        f"startup_channel_probe label={label} channel={channel_id} status=ok source={source} type={type(channel).__name__} parent={parent_text} messageable={messageable} allowed_message={allowed_message}"
    )


async def log_startup_diagnostics(deps: StartupDiagnosticsDeps) -> None:
    try:
        targets = deps.get_probe_targets(deps.allowed_channel_ids, deps.startup_channel_id)
        deps.log(f"startup_diagnostics_start targets={len(targets)}")
        probe_timeout = deps.get_probe_timeout()
        for label, channel_id in targets:
            try:
                await asyncio.wait_for(
                    deps.probe_channel_access(label, channel_id),
                    timeout=probe_timeout,
                )
            except TimeoutError:
                deps.log(
                    f"startup_channel_probe label={label} channel={channel_id} "
                    + f"status=timeout timeout_seconds={probe_timeout:g}"
                )
        deps.log("startup_diagnostics_done")
    except deps.delivery_exceptions:
        deps.log("startup_diagnostics_failed\n" + deps.format_traceback())

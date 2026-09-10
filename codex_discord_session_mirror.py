"""Session mirror event extraction helpers."""

from __future__ import annotations

from asyncio import CancelledError  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass, field

from codex_discord_session_mirror_item_builders import (
    SessionEvent,
    SessionMirrorItem,
    SessionPayload,
    TextDigestFunc,
    format_aborted_event_text,
    format_session_mirror_text,
    has_recent_session_text,
    make_session_mirror_event_digest,
    make_session_mirror_item,
    make_text_digest,
    remember_recent_session_text,
)
from codex_discord_session_mirror_item_append import (
    BuildInteractiveNoticeFunc,
    ExtractMessageTextFunc,
    SkipDiscordOriginPromptFunc,
)
from codex_discord_session_mirror_item_collection import (
    collect_session_mirror_items,
)

SessionMirrorTargetValue = str | int | bytes | bytearray | None
SessionMirrorTargetMapping = Mapping[str, SessionMirrorTargetValue]
LoadSessionMirrorTargets = Callable[[], Awaitable[Sequence[SessionMirrorTargetMapping]]]
MirrorSessionTarget = Callable[[SessionMirrorTargetMapping], Awaitable[None]]
LogFunc = Callable[[str], None]
SleepFunc = Callable[[float], Awaitable[None]]
SetLastAtFunc = Callable[[str], None]
NowIsoFunc = Callable[[], str]
TracebackFormatter = Callable[[], str]

__all__ = (
    "BuildInteractiveNoticeFunc",
    "ExtractMessageTextFunc",
    "SessionEvent",
    "SessionMirrorLoopDeps",
    "SessionMirrorItem",
    "SessionMirrorState",
    "SessionMirrorTarget",
    "SessionMirrorTargetMapping",
    "SessionMirrorTargetValue",
    "SessionPayload",
    "SkipDiscordOriginPromptFunc",
    "TextDigestFunc",
    "collect_session_mirror_items",
    "format_aborted_event_text",
    "format_session_mirror_text",
    "has_recent_session_text",
    "make_session_mirror_event_digest",
    "make_session_mirror_item",
    "make_text_digest",
    "parse_session_mirror_target",
    "remember_recent_session_text",
    "session_mirror_loop",
)

@dataclass(frozen=True, slots=True)
class SessionMirrorTarget:
    codex_thread_id: str
    discord_thread_id: int


@dataclass(slots=True)  # noqa: MUTABLE_OK
class SessionMirrorState:
    """Mutable tracking state for session mirror output cursors."""

    active_output_targets: dict[str, float] = field(default_factory=dict)
    pending_cursor_targets: set[str] = field(default_factory=set)
    expiring_output_targets: dict[str, float] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class SessionMirrorLoopDeps:
    poll_seconds: float
    is_closed: Callable[[], bool]
    set_last_at: SetLastAtFunc
    now_iso: NowIsoFunc
    load_targets: LoadSessionMirrorTargets
    mirror_session_target: MirrorSessionTarget
    delivery_exceptions: tuple[type[BaseException], ...]
    format_traceback: TracebackFormatter
    sleep: SleepFunc
    log: LogFunc


async def session_mirror_loop(deps: SessionMirrorLoopDeps) -> None:
    while not deps.is_closed():
        try:
            deps.set_last_at(deps.now_iso())
            for target in await deps.load_targets():
                await deps.mirror_session_target(target)
        except CancelledError:
            raise
        except deps.delivery_exceptions:
            deps.log("session_mirror_cycle_failed\n" + deps.format_traceback())
        except Exception:  # noqa: BROAD_EXCEPT_OK - a background poller must survive unexpected cycle defects.
            deps.log("session_mirror_unexpected_error\n" + deps.format_traceback())
        await deps.sleep(deps.poll_seconds)


def parse_session_mirror_target(target: Mapping[str, SessionMirrorTargetValue]) -> SessionMirrorTarget | None:
    codex_thread_id = str(target.get("codex_thread_id") or "")
    if not codex_thread_id:
        return None
    raw_discord_thread_id = target.get("discord_thread_id")
    if isinstance(raw_discord_thread_id, bytes | bytearray):
        discord_thread_id_text = bytes(raw_discord_thread_id).decode("ascii", errors="ignore")
    else:
        discord_thread_id_text = str(raw_discord_thread_id or 0)
    try:
        discord_thread_id = int(discord_thread_id_text)
    except (TypeError, ValueError):
        return None
    if not discord_thread_id:
        return None
    return SessionMirrorTarget(
        codex_thread_id=codex_thread_id,
        discord_thread_id=discord_thread_id,
    )

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, Protocol, TypeAlias, TypeVar

import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_delivery_flow as delivery_flow
import codex_discord_session_mirror_event_flow as event_flow
import codex_discord_session_mirror_item_sender as item_sender
import codex_discord_session_mirror_cursor as session_mirror_cursor
import codex_pro_session_mirror_output as pro_session_mirror_output
from codex_app_server_transport_goal import GoalAbsent, GoalPresent, GoalTransportError, ThreadGoalLookup
from codex_app_server_transport_goal import is_terminal_goal_status

ThreadT = TypeVar("ThreadT")
ThreadT_co = TypeVar("ThreadT_co", covariant=True)
ContextUsageT = TypeVar("ContextUsageT")
EventT = TypeVar("EventT")
ChannelT = TypeVar("ChannelT")

SessionMirrorTargetValue: TypeAlias = session_mirror.SessionMirrorTargetValue
SessionMirrorTargetMapping: TypeAlias = Mapping[str, SessionMirrorTargetValue]
SessionMirrorItem: TypeAlias = item_sender.SessionMirrorItem


class ChooseThreadSync(Protocol[ThreadT_co]):
    def __call__(self, thread_id: str | None, cwd: str | None) -> ThreadT_co: ...


class ReadNewSessionEventsSync(Protocol[EventT]):
    def __call__(
        self,
        session_path: Path,
        cursor: int,
        *,
        max_events: int | None = None,
    ) -> tuple[list[EventT], int]: ...


class SessionMirrorItemCollector(Protocol[EventT]):
    def __call__(
        self,
        codex_thread_id: str,
        events: list[EventT],
        *,
        seen_agent_messages: dict[str, float],
        seen_user_messages: dict[str, float],
    ) -> list[SessionMirrorItem]: ...


async def noop_send_typing_pulse(channel: object, target_thread_id: str, context: str) -> None:
    _ = channel
    _ = target_thread_id
    _ = context


def default_thread_busy(session_path: Path) -> bool:
    _ = session_path
    return True


@dataclass(frozen=True, slots=True)
class SessionMirrorTargetDeps(Generic[ThreadT, ContextUsageT, EventT, ChannelT]):
    parse_session_mirror_target: Callable[
        [SessionMirrorTargetMapping],
        session_mirror.SessionMirrorTarget | None,
    ]
    choose_thread: ChooseThreadSync[ThreadT]
    get_thread_context_usage: Callable[[ThreadT], ContextUsageT]
    should_recommend_archive: Callable[[ThreadT, ContextUsageT], bool]
    get_thread_rollout_path: Callable[[ThreadT], str]
    is_active_output_target: Callable[[str], bool]
    archive_skip_logged: set[str]
    is_pending_cursor_target: session_mirror_cursor.PendingCursorPredicate
    clear_pending_cursor_target: session_mirror_cursor.PendingCursorClearer
    update_session_mirror_cursor: Callable[[str, str, int], None]
    get_or_init_session_mirror_cursor: Callable[[str, str, int], int]
    read_new_session_events: ReadNewSessionEventsSync[EventT]
    get_archive_backlog_max_events: Callable[[], int]
    collect_session_mirror_items: SessionMirrorItemCollector[EventT]
    get_seen_agent_messages: Callable[[str], dict[str, float]]
    get_seen_user_messages: Callable[[str], dict[str, float]]
    resolve_session_mirror_channel: Callable[[int], Awaitable[ChannelT | None]]
    resolve_target_ref: Callable[[str], tuple[str | None, str]]
    has_session_mirror_event: Callable[[str, str], bool]
    send_session_mirror_item: item_sender.SessionMirrorItemSender[ChannelT]
    claim_session_mirror_event: Callable[[str, str], bool]
    release_session_mirror_output_target: Callable[[str], Awaitable[bool]]
    log: Callable[[str], None]
    send_typing_pulse: Callable[[ChannelT, str, str], Awaitable[None]] = noop_send_typing_pulse
    is_thread_busy: Callable[[Path], bool] = default_thread_busy
    get_active_turn_id: Callable[[str], str | None] = lambda thread_id: None
    get_thread_goal_lookup: Callable[[str], ThreadGoalLookup] = lambda thread_id: GoalAbsent()


async def _update_cursor(
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
    codex_thread_id: str,
    rollout_path: str,
    cursor: int,
) -> None:
    await asyncio.to_thread(deps.update_session_mirror_cursor, codex_thread_id, rollout_path, cursor)


async def _get_or_init_cursor(
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
    codex_thread_id: str,
    rollout_path: str,
    initial_cursor: int,
) -> int:
    return await asyncio.to_thread(
        deps.get_or_init_session_mirror_cursor, codex_thread_id, rollout_path, initial_cursor
    )


async def _read_events(
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
    session_path: Path,
    cursor: int,
    max_events: int | None,
) -> tuple[list[EventT], int]:
    if max_events is None:
        return await asyncio.to_thread(deps.read_new_session_events, session_path, cursor)
    return await asyncio.to_thread(
        deps.read_new_session_events,
        session_path,
        cursor,
        max_events=max_events,
    )


async def _send_typing_pulse_if_busy(
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
    codex_thread_id: str,
    discord_thread_id: int,
) -> None:
    if not deps.is_active_output_target(codex_thread_id):
        return
    channel = await deps.resolve_session_mirror_channel(discord_thread_id)
    if channel is None:
        return
    await deps.send_typing_pulse(channel, codex_thread_id, "session_mirror_busy")
    deps.log(f"session_mirror_typing_pulse target={codex_thread_id} channel={discord_thread_id}")


async def _deactivate_output_target_if_idle(
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
    codex_thread_id: str,
) -> bool:
    if not deps.is_active_output_target(codex_thread_id):
        return False
    try:
        codex_thread = await asyncio.to_thread(deps.choose_thread, codex_thread_id, None)
    except Exception:  # noqa: BROAD_EXCEPT_OK - mirror loop already logs unavailable threads upstream.
        return False
    session_path = Path(deps.get_thread_rollout_path(codex_thread))
    if not session_path.exists():
        return False
    if await asyncio.to_thread(deps.is_thread_busy, session_path):
        return False
    try:
        active_turn_id = await asyncio.to_thread(deps.get_active_turn_id, codex_thread_id)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK - transport failures keep the watcher alive.
        deps.log(
            f"session_mirror_active_turn_lookup_failed target={codex_thread_id} "
            + f"error_type={type(exc).__name__}"
        )
        return False
    if active_turn_id:
        return False
    goal_lookup = await asyncio.to_thread(deps.get_thread_goal_lookup, codex_thread_id)
    if isinstance(goal_lookup, GoalTransportError):
        deps.log(f"session_mirror_goal_lookup_failed target={codex_thread_id}")
        return False
    if isinstance(goal_lookup, GoalPresent) and not is_terminal_goal_status(goal_lookup.status):
        return False
    if not await deps.release_session_mirror_output_target(codex_thread_id):
        return False
    deps.log(f"session_mirror_output_deactivated_idle target={codex_thread_id}")
    return True


async def mirror_session_target(
    target: SessionMirrorTargetMapping,
    *,
    deps: SessionMirrorTargetDeps[ThreadT, ContextUsageT, EventT, ChannelT],
) -> None:
    mirror_target = deps.parse_session_mirror_target(target)
    if mirror_target is None:
        return
    codex_thread_id = mirror_target.codex_thread_id
    discord_thread_id = mirror_target.discord_thread_id

    if await pro_session_mirror_output.gate_session_mirror_output(deps, codex_thread_id):
        return

    prepared_items = await event_flow.prepare_session_mirror_delivery_items(
        codex_thread_id,
        deps=event_flow.SessionMirrorEventFlowDeps(
            choose_thread=lambda codex_thread_id: asyncio.to_thread(
                deps.choose_thread,
                codex_thread_id,
                None,
            ),
            get_thread_context_usage=lambda codex_thread: asyncio.to_thread(
                deps.get_thread_context_usage,
                codex_thread,
            ),
            should_recommend_archive=deps.should_recommend_archive,
            get_thread_rollout_path=deps.get_thread_rollout_path,
            is_active_output_target=deps.is_active_output_target,
            archive_skip_logged=deps.archive_skip_logged,
            is_pending_cursor_target=deps.is_pending_cursor_target,
            clear_pending_cursor_target=deps.clear_pending_cursor_target,
            update_session_mirror_cursor=lambda codex_thread_id, rollout_path, cursor: _update_cursor(
                deps,
                codex_thread_id,
                rollout_path,
                cursor,
            ),
            get_or_init_session_mirror_cursor=lambda codex_thread_id, rollout_path, initial_cursor: _get_or_init_cursor(
                deps,
                codex_thread_id,
                rollout_path,
                initial_cursor,
            ),
            read_events=lambda session_path, cursor, max_events: _read_events(
                deps,
                session_path,
                cursor,
                max_events,
            ),
            get_archive_backlog_max_events=deps.get_archive_backlog_max_events,
            collect_session_mirror_items=deps.collect_session_mirror_items,
            get_seen_agent_messages=deps.get_seen_agent_messages,
            get_seen_user_messages=deps.get_seen_user_messages,
            log=deps.log,
        ),
    )
    if prepared_items is None:
        if await _deactivate_output_target_if_idle(deps, codex_thread_id):
            return
        await _send_typing_pulse_if_busy(deps, codex_thread_id, discord_thread_id)
        return

    _ = await delivery_flow.deliver_and_commit_session_mirror_items(
        codex_thread_id,
        prepared_items.rollout_path,
        prepared_items.next_cursor,
        discord_thread_id=discord_thread_id,
        event_count=len(prepared_items.events),
        items=prepared_items.items,
        deps=delivery_flow.SessionMirrorDeliveryFlowDeps(
            resolve_session_mirror_channel=deps.resolve_session_mirror_channel,
            resolve_target_ref=deps.resolve_target_ref,
            has_session_mirror_event=lambda digest, codex_thread_id: asyncio.to_thread(
                deps.has_session_mirror_event,
                digest,
                codex_thread_id,
            ),
            send_session_mirror_item=deps.send_session_mirror_item,
            claim_session_mirror_event=lambda digest, codex_thread_id: asyncio.to_thread(
                deps.claim_session_mirror_event,
                digest,
                codex_thread_id,
            ),
            update_session_mirror_cursor=lambda codex_thread_id, rollout_path, cursor: _update_cursor(
                deps,
                codex_thread_id,
                rollout_path,
                cursor,
            ),
            release_session_mirror_output_target=deps.release_session_mirror_output_target,
            log=deps.log,
        ),
    )

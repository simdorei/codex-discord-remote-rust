from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Coroutine, Sequence
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Generic, Protocol, TypeAlias, TypeVar

import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_channels as discord_session_mirror_channels
import codex_discord_session_mirror_item_delivery as discord_session_mirror_item_delivery
import codex_discord_session_mirror_target as discord_session_mirror_target
from codex_session_events import JsonEvent
from codex_thread_models import ThreadContextUsage, ThreadInfo
from codex_app_server_transport_goal import GoalAbsent, ThreadGoalLookup
ModuleValue: TypeAlias = object

ChannelT = TypeVar("ChannelT")
SessionMirrorTargetMapping: TypeAlias = discord_session_mirror.SessionMirrorTargetMapping
LoadTargetsInThread = Callable[[Path, int], Awaitable[Sequence[SessionMirrorTargetMapping]]]
CreateTaskFunc = Callable[[Coroutine[object, object, None]], asyncio.Task[None]]
LogFunc = Callable[[str], None]


async def noop_send_typing_pulse(channel: object, target_thread_id: str, context: str) -> None:
    _ = channel
    _ = target_thread_id
    _ = context


async def noop_deliver_pending_approval(owner: object, target: SessionMirrorTargetMapping) -> bool:
    _ = owner
    _ = target
    return False


class SessionMirrorOwner(Protocol[ChannelT]):
    session_mirror_poll_seconds: float

    def is_closed(self) -> bool: ...

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[ChannelT | None, str]: ...

    async def fetch_channel(self, channel_id: int) -> ChannelT: ...

    def get_session_mirror_seen_agent_messages(self, codex_thread_id: str) -> dict[str, float]: ...

    def get_session_mirror_seen_user_messages(self, codex_thread_id: str) -> dict[str, float]: ...

    async def session_mirror_loop(self) -> None: ...

    async def resolve_session_mirror_channel(self, discord_thread_id: int) -> ChannelT | None: ...

    async def send_session_mirror_item(
        self,
        channel: ChannelT,
        item: discord_session_mirror_item_delivery.SessionMirrorItem,
        *,
        target_thread_id: str,
        target_ref: str,
    ) -> None: ...

    async def mirror_session_target(self, target: SessionMirrorTargetMapping) -> None: ...


@dataclass(frozen=True, slots=True)
class SessionMirrorRuntimeDeps(Generic[ChannelT]):
    mirror_enabled: Callable[[], bool]
    target_limit: int
    delivery_exceptions: tuple[type[BaseException], ...]
    fetch_failure_types: tuple[type[Exception], ...]
    get_db_path: Callable[[], Path]
    load_targets_in_thread: LoadTargetsInThread
    create_task: CreateTaskFunc
    sleep: Callable[[float], Awaitable[None]]
    now_iso: Callable[[], str]
    format_traceback: Callable[[], str]
    is_messageable: Callable[[ChannelT], bool]
    parse_interactive_notice: discord_session_mirror_item_delivery.ParseInteractiveNotice
    send_interactive_prompt: discord_session_mirror_item_delivery.SessionMirrorInteractiveSender[ChannelT]
    send_chunks: discord_session_mirror_item_delivery.SessionMirrorChunkSender[ChannelT]
    send_attachment: discord_session_mirror_item_delivery.SessionMirrorAttachmentSender[ChannelT]
    format_session_mirror_text: discord_session_mirror_item_delivery.FormatSessionMirrorText
    parse_session_mirror_target: Callable[
        [SessionMirrorTargetMapping],
        discord_session_mirror.SessionMirrorTarget | None,
    ]
    choose_thread: discord_session_mirror_target.ChooseThreadSync[ThreadInfo]
    get_thread_context_usage: Callable[[ThreadInfo], ThreadContextUsage]
    should_recommend_archive: Callable[[ThreadInfo, ThreadContextUsage], bool]
    get_thread_rollout_path: Callable[[ThreadInfo], str]
    is_active_output_target: Callable[[str], bool]
    is_pending_cursor_target: Callable[[str], bool]
    clear_pending_cursor_target: Callable[[str], None]
    update_session_mirror_cursor: Callable[[str, str, int], None]
    get_or_init_session_mirror_cursor: Callable[[str, str, int], int]
    read_new_session_events: discord_session_mirror_target.ReadNewSessionEventsSync[JsonEvent]
    get_archive_backlog_max_events: Callable[[], int]
    collect_session_mirror_items: discord_session_mirror_target.SessionMirrorItemCollector[JsonEvent]
    get_archive_skip_logged: Callable[[SessionMirrorOwner[ChannelT]], set[str]]
    resolve_target_ref: Callable[[str], tuple[str | None, str]]
    has_session_mirror_event: Callable[[str, str], bool]
    claim_session_mirror_event: Callable[[str, str], bool]
    release_session_mirror_output_target: Callable[[str], Awaitable[bool]]
    log: LogFunc
    send_typing_pulse: Callable[[ChannelT, str, str], Awaitable[None]] = noop_send_typing_pulse
    is_thread_busy: Callable[[Path], bool] = lambda session_path: True
    get_active_turn_id: Callable[[str], str | None] = lambda thread_id: None
    get_thread_goal_lookup: Callable[[str], ThreadGoalLookup] = lambda thread_id: GoalAbsent()
    deliver_pending_approval: Callable[
        [SessionMirrorOwner[ChannelT], SessionMirrorTargetMapping],
        Awaitable[bool],
    ] = noop_deliver_pending_approval


@dataclass(frozen=True, slots=True)
class SessionMirrorRuntime(Generic[ChannelT]):
    deps: SessionMirrorRuntimeDeps[ChannelT]

    async def start_session_mirroring(self, owner: SessionMirrorOwner[ChannelT]) -> None:
        if not self.deps.mirror_enabled():
            self.deps.log("session_mirror_disabled")
            return
        task = _session_mirror_task(owner)
        if task and not task.done():
            self.deps.log("session_mirror_already_running")
            return
        setattr(owner, "_session_mirror_task", self.deps.create_task(owner.session_mirror_loop()))
        self.deps.log(f"session_mirror_started seconds={owner.session_mirror_poll_seconds:g}")

    async def session_mirror_loop(self, owner: SessionMirrorOwner[ChannelT]) -> None:
        async def load_targets() -> Sequence[SessionMirrorTargetMapping]:
            return await self.deps.load_targets_in_thread(
                self.deps.get_db_path(),
                self.deps.target_limit,
            )

        await discord_session_mirror.session_mirror_loop(
            discord_session_mirror.SessionMirrorLoopDeps(
                poll_seconds=owner.session_mirror_poll_seconds,
                is_closed=owner.is_closed,
                set_last_at=lambda value: setattr(owner, "_session_mirror_last_at", value),
                now_iso=self.deps.now_iso,
                load_targets=load_targets,
                mirror_session_target=owner.mirror_session_target,
                delivery_exceptions=self.deps.delivery_exceptions,
                format_traceback=self.deps.format_traceback,
                sleep=self.deps.sleep,
                log=self.deps.log,
            )
        )

    async def resolve_session_mirror_channel(
        self,
        owner: SessionMirrorOwner[ChannelT],
        discord_thread_id: int,
    ) -> ChannelT | None:
        channel = await discord_session_mirror_channels.resolve_session_mirror_channel(
            int(discord_thread_id),
            deps=discord_session_mirror_channels.SessionMirrorChannelResolveDeps(
                get_cached_channel_or_thread=owner.get_cached_channel_or_thread,
                fetch_channel=owner.fetch_channel,
                fetch_failure_types=self.deps.fetch_failure_types,
                is_messageable=self.deps.is_messageable,
                log=self.deps.log,
            ),
        )
        if channel is None:
            return None
        return channel if self.deps.is_messageable(channel) else None

    async def send_session_mirror_item(
        self,
        channel: ChannelT,
        item: discord_session_mirror_item_delivery.SessionMirrorItem,
        *,
        target_thread_id: str,
        target_ref: str,
    ) -> None:
        await discord_session_mirror_item_delivery.send_session_mirror_item(
            channel,
            item,
            target_thread_id=target_thread_id,
            target_ref=target_ref,
            deps=discord_session_mirror_item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=self.deps.parse_interactive_notice,
                send_interactive_prompt=self.deps.send_interactive_prompt,
                send_chunks=self.deps.send_chunks,
                send_attachment=self.deps.send_attachment,
                format_session_mirror_text=self.deps.format_session_mirror_text,
            ),
        )

    async def mirror_session_target(
        self,
        owner: SessionMirrorOwner[ChannelT],
        target: SessionMirrorTargetMapping,
    ) -> None:
        if await self.deps.deliver_pending_approval(owner, target):
            return
        deps: discord_session_mirror_target.SessionMirrorTargetDeps[
            ThreadInfo,
            ThreadContextUsage,
            JsonEvent,
            ChannelT,
        ] = discord_session_mirror_target.SessionMirrorTargetDeps(
            parse_session_mirror_target=self.deps.parse_session_mirror_target,
            choose_thread=self.deps.choose_thread,
            get_thread_context_usage=self.deps.get_thread_context_usage,
            should_recommend_archive=self.deps.should_recommend_archive,
            get_thread_rollout_path=self.deps.get_thread_rollout_path,
            is_active_output_target=self.deps.is_active_output_target,
            archive_skip_logged=self.deps.get_archive_skip_logged(owner),
            is_pending_cursor_target=self.deps.is_pending_cursor_target,
            clear_pending_cursor_target=self.deps.clear_pending_cursor_target,
            update_session_mirror_cursor=self.deps.update_session_mirror_cursor,
            get_or_init_session_mirror_cursor=self.deps.get_or_init_session_mirror_cursor,
            read_new_session_events=self.deps.read_new_session_events,
            get_archive_backlog_max_events=self.deps.get_archive_backlog_max_events,
            collect_session_mirror_items=self.deps.collect_session_mirror_items,
            get_seen_agent_messages=owner.get_session_mirror_seen_agent_messages,
            get_seen_user_messages=owner.get_session_mirror_seen_user_messages,
            resolve_session_mirror_channel=owner.resolve_session_mirror_channel,
            resolve_target_ref=self.deps.resolve_target_ref,
            has_session_mirror_event=self.deps.has_session_mirror_event,
            send_session_mirror_item=owner.send_session_mirror_item,
            claim_session_mirror_event=self.deps.claim_session_mirror_event,
            release_session_mirror_output_target=self.deps.release_session_mirror_output_target,
            send_typing_pulse=lambda channel, target_thread_id, context: self.deps.send_typing_pulse(
                channel,
                target_thread_id,
                context,
            ),
            is_thread_busy=self.deps.is_thread_busy,
            get_active_turn_id=self.deps.get_active_turn_id,
            get_thread_goal_lookup=self.deps.get_thread_goal_lookup,
            log=self.deps.log,
        )
        await discord_session_mirror_target.mirror_session_target(target, deps=deps)

def _session_mirror_task(owner: SessionMirrorOwner[ChannelT]) -> asyncio.Task[ModuleValue] | None:
    value = getattr(owner, "_session_mirror_task", None)
    if isinstance(value, asyncio.Task):
        return value
    return None


def utc_now_iso_seconds() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")

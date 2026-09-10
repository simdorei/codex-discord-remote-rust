from __future__ import annotations

import os
import traceback
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Protocol, TypeVar

import codex_app_server_transport as app_server_transport
import codex_discord_bot_session_mirror_runtime as session_mirror_runtime
import codex_discord_pending_approval_delivery as pending_approval_delivery
import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_item_delivery as discord_session_mirror_item_delivery
import codex_discord_session_mirror_target as discord_session_mirror_target
from codex_discord_text import env_flag, parse_bounded_int_arg
from codex_session_events import JsonEvent
from codex_thread_models import ThreadContextUsage, ThreadInfo


ChannelT = TypeVar("ChannelT")


class SessionMirrorEventsBridge(Protocol):
    choose_thread: discord_session_mirror_target.ChooseThreadSync[ThreadInfo]
    read_new_session_events: discord_session_mirror_target.ReadNewSessionEventsSync[JsonEvent]

    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsage: ...

    def should_recommend_archive(self, thread: ThreadInfo, usage: ThreadContextUsage) -> bool: ...

    def is_thread_busy(self, session_path: Path) -> bool: ...


def make_session_mirror_runtime(
    *,
    target_limit: int,
    archive_backlog_max_events_default: int,
    delivery_exceptions: tuple[type[BaseException], ...],
    fetch_failure_types: tuple[type[Exception], ...],
    get_db_path: Callable[[], Path],
    load_targets_in_thread: session_mirror_runtime.LoadTargetsInThread,
    create_task: session_mirror_runtime.CreateTaskFunc,
    sleep: Callable[[float], Awaitable[None]],
    is_messageable: Callable[[ChannelT], bool],
    parse_interactive_notice: discord_session_mirror_item_delivery.ParseInteractiveNotice,
    send_interactive_prompt: discord_session_mirror_item_delivery.SessionMirrorInteractiveSender[ChannelT],
    send_chunks: discord_session_mirror_item_delivery.SessionMirrorChunkSender[ChannelT],
    send_attachment: discord_session_mirror_item_delivery.SessionMirrorAttachmentSender[ChannelT],
    collect_session_mirror_items: discord_session_mirror_target.SessionMirrorItemCollector[JsonEvent],
    get_archive_skip_logged: Callable[[session_mirror_runtime.SessionMirrorOwner[ChannelT]], set[str]],
    resolve_target_ref: Callable[[str], tuple[str | None, str]],
    is_active_output_target: Callable[[str], bool],
    is_pending_cursor_target: Callable[[str], bool],
    clear_pending_cursor_target: Callable[[str], None],
    update_session_mirror_cursor: Callable[[str, str, int], None],
    get_or_init_session_mirror_cursor: Callable[[str, str, int], int],
    has_session_mirror_event: Callable[[str, str], bool],
    claim_session_mirror_event: Callable[[str, str], bool],
    release_session_mirror_output_target: Callable[[str], Awaitable[bool]],
    events_bridge: SessionMirrorEventsBridge,
    log: Callable[[str], None],
    send_typing_pulse: Callable[[ChannelT, str, str], Awaitable[None]] | None = None,
) -> session_mirror_runtime.SessionMirrorRuntime[ChannelT]:
    def app_server_enabled() -> bool:
        return env_flag("CODEX_DISCORD_APP_SERVER_TRANSPORT", True)

    def healthy_app_server_generation() -> tuple[int | None, app_server_transport.AppServerLifecycleSnapshot]:
        snapshot = app_server_transport.DEFAULT_CLIENT.lifecycle_snapshot()
        generation = snapshot.generation if snapshot.healthy and snapshot.generation > 0 else None
        return generation, snapshot

    def get_active_turn_id(thread_id: str) -> str | None:
        if not app_server_enabled():
            return None
        generation, snapshot = healthy_app_server_generation()
        if generation is None:
            raise app_server_transport.AppServerGenerationMismatch(
                expected_generation=snapshot.generation,
                actual_generation=snapshot.generation,
                healthy=False,
            )
        return app_server_transport.DEFAULT_CLIENT.get_active_turn_id_or_raise(
            thread_id,
            expected_generation=generation,
        )

    def get_thread_goal_lookup(thread_id: str) -> app_server_transport.ThreadGoalLookup:
        if not app_server_enabled():
            return app_server_transport.GoalAbsent()
        generation, _snapshot = healthy_app_server_generation()
        if generation is None:
            return app_server_transport.GoalTransportError("Codex app-server is not healthy.")
        return app_server_transport.DEFAULT_CLIENT.get_thread_goal_lookup(
            thread_id,
            expected_generation=generation,
        )

    def read_new_session_events(
        session_path: Path,
        cursor: int,
        *,
        max_events: int | None = None,
    ) -> tuple[list[JsonEvent], int]:
        if max_events is None:
            return events_bridge.read_new_session_events(session_path, cursor)
        return events_bridge.read_new_session_events(session_path, cursor, max_events=max_events)

    return session_mirror_runtime.SessionMirrorRuntime(
        session_mirror_runtime.SessionMirrorRuntimeDeps(
            mirror_enabled=lambda: env_flag("DISCORD_SESSION_MIRROR", default=True),
            target_limit=target_limit,
            delivery_exceptions=delivery_exceptions,
            fetch_failure_types=fetch_failure_types,
            get_db_path=get_db_path,
            load_targets_in_thread=load_targets_in_thread,
            create_task=create_task,
            sleep=sleep,
            now_iso=session_mirror_runtime.utc_now_iso_seconds,
            format_traceback=traceback.format_exc,
            is_messageable=is_messageable,
            parse_interactive_notice=parse_interactive_notice,
            send_interactive_prompt=send_interactive_prompt,
            send_chunks=send_chunks,
            send_attachment=send_attachment,
            format_session_mirror_text=format_session_mirror_delivery_text,
            parse_session_mirror_target=discord_session_mirror.parse_session_mirror_target,
            choose_thread=lambda thread_id, cwd: events_bridge.choose_thread(thread_id, cwd),
            get_thread_context_usage=lambda thread: events_bridge.get_thread_context_usage(thread),
            should_recommend_archive=lambda thread, usage: events_bridge.should_recommend_archive(thread, usage),
            get_thread_rollout_path=lambda codex_thread: str(codex_thread.rollout_path),
            is_active_output_target=is_active_output_target,
            is_pending_cursor_target=is_pending_cursor_target,
            clear_pending_cursor_target=clear_pending_cursor_target,
            update_session_mirror_cursor=update_session_mirror_cursor,
            get_or_init_session_mirror_cursor=get_or_init_session_mirror_cursor,
            read_new_session_events=read_new_session_events,
            get_archive_backlog_max_events=lambda: parse_bounded_int_arg(
                os.environ.get("DISCORD_SESSION_MIRROR_ARCHIVE_BACKLOG_MAX_EVENTS", ""),
                default=archive_backlog_max_events_default,
                minimum=0,
                maximum=10000,
            ),
            collect_session_mirror_items=collect_session_mirror_items,
            get_archive_skip_logged=get_archive_skip_logged,
            resolve_target_ref=resolve_target_ref,
            has_session_mirror_event=has_session_mirror_event,
            claim_session_mirror_event=claim_session_mirror_event,
            release_session_mirror_output_target=release_session_mirror_output_target,
            log=log,
            send_typing_pulse=send_typing_pulse or session_mirror_runtime.noop_send_typing_pulse,
            is_thread_busy=lambda session_path: events_bridge.is_thread_busy(session_path),
            get_active_turn_id=get_active_turn_id,
            get_thread_goal_lookup=get_thread_goal_lookup,
            deliver_pending_approval=lambda owner, target: pending_approval_delivery.deliver_pending_approval(
                owner,
                target,
                deps=pending_approval_delivery.PendingApprovalDeliveryDeps(
                    parse_target=discord_session_mirror.parse_session_mirror_target,
                    get_pending_request=lambda thread_id: (
                        app_server_transport.DEFAULT_CLIENT.get_latest_pending_approval_request(thread_id)
                        if app_server_enabled()
                        else None
                    ),
                    has_delivered=has_session_mirror_event,
                    claim_delivery=claim_session_mirror_event,
                    resolve_target_ref=resolve_target_ref,
                    log=log,
                ),
            ),
        )
    )


def format_session_mirror_delivery_text(
    item: discord_session_mirror_item_delivery.SessionMirrorItem,
) -> str:
    return discord_session_mirror.format_session_mirror_text(dict(item))

"""Discord diagnostics and history message builders."""

from __future__ import annotations

import sqlite3
from collections.abc import Awaitable, Callable, Mapping
from typing import Literal, Protocol, TypeAlias, runtime_checkable

from codex_discord_diagnostics_history import (
    CachedChannelGetter as CachedChannelGetter,
    ClientChannelGetter as ClientChannelGetter,
    DiscordHistoryChannel as DiscordHistoryChannel,
    FetchChannelGetter as FetchChannelGetter,
    LogTextLengthFormatter as LogTextLengthFormatter,
    StartupProbeTargetsGetter as StartupProbeTargetsGetter,
    build_discord_channel_history_lines as build_discord_channel_history_lines,
    build_discord_tracked_target_user_history_lines as build_discord_tracked_target_user_history_lines,
    format_discord_message_created_at as format_discord_message_created_at,
    format_discord_message_type as format_discord_message_type,
    resolve_discord_history_channel as resolve_discord_history_channel,
)


DISCORD_DOCTOR_LOG_MARKER_KEYS = (
    "last_ready_at", "last_gateway_event_at", "last_raw_interaction_at", "last_interaction_at", "last_component_at",
    "last_user_or_control_hook_at", "last_button_qa_at", "last_button_qa_result", "last_steering_button_at",
    "last_steering_button_exit", "last_steering_button_elapsed_sec",
)


class DiagnosticBotLike(Protocol):
    pass


class DiagnosticChannelLike(Protocol):
    pass


@runtime_checkable
class DoneTaskLike(Protocol):
    def done(self) -> bool: ...


@runtime_checkable
class MessageContentIntentLike(Protocol):
    @property
    def message_content(self) -> bool: ...


DiagnosticMarkerValue: TypeAlias = str
DiagnosticAttrValue: TypeAlias = set[int] | str | int | bool | DoneTaskLike | MessageContentIntentLike | None
BotIntSetAttrName: TypeAlias = Literal[
    "_history_poll_primed_channels", "allowed_channel_ids", "allowed_user_ids"
]
MirroredThreadIdGetter: TypeAlias = Callable[[int | None], str | None]
MirrorProjectGetter: TypeAlias = Callable[[int | None], tuple[str, str] | None]
CountGetter: TypeAlias = Callable[[], tuple[int, int]]
MirrorCheckBuilder: TypeAlias = Callable[[], str]
DiscordLogMarkersGetter: TypeAlias = Callable[[], Mapping[str, DiagnosticMarkerValue]]
RecentDiscordHookEventsGetter: TypeAlias = Callable[..., list[str]]
QaCommandsEnabledGetter: TypeAlias = Callable[[], bool]
DoctorMessageBuilder: TypeAlias = Callable[..., str]
ChannelHistoryBuilder: TypeAlias = Callable[..., Awaitable[list[str]]]
TrackedTargetHistoryBuilder: TypeAlias = Callable[..., Awaitable[list[str]]]


def _bot_int_set_attr(bot: DiagnosticBotLike, name: BotIntSetAttrName) -> set[int]:
    value: DiagnosticAttrValue = getattr(bot, name, None)
    if isinstance(value, set):
        return value
    return set()


def format_discord_id_list(values: set[int], *, limit: int = 8) -> str:
    if not values:
        return "ALL"
    sorted_values = sorted(values)
    rendered = ",".join(str(value) for value in sorted_values[:limit])
    if len(sorted_values) > limit:
        rendered += f",+{len(sorted_values) - limit} more"
    return rendered


def build_discord_doctor_log_marker_lines(log_markers: Mapping[str, DiagnosticMarkerValue]) -> list[str]:
    return [f"{key}: {log_markers[key]}" for key in DISCORD_DOCTOR_LOG_MARKER_KEYS]


def build_discord_doctor_message(
    bot: DiagnosticBotLike,
    channel_id: int | None,
    *,
    empty_content_notice_count: int,
    get_mirrored_codex_thread_id_func: MirroredThreadIdGetter,
    get_mirror_project_for_channel_func: MirrorProjectGetter,
    get_busy_choice_counts_func: CountGetter,
    get_persistent_component_claim_counts_func: CountGetter,
    build_mirror_check_func: MirrorCheckBuilder,
    get_discord_log_markers_func: DiscordLogMarkersGetter,
    get_recent_discord_hook_events_func: RecentDiscordHookEventsGetter,
    discord_qa_commands_enabled_func: QaCommandsEnabledGetter,
) -> str:
    target_thread_id = get_mirrored_codex_thread_id_func(channel_id)
    project = get_mirror_project_for_channel_func(channel_id)
    active_busy_choices, stale_busy_choices = get_busy_choice_counts_func()
    active_component_claims, stale_component_claims = get_persistent_component_claim_counts_func()
    try:
        mirror_lines = build_mirror_check_func().splitlines()
    except (RuntimeError, OSError, sqlite3.Error) as exc:
        mirror_lines = ["Mirror check failed", f"ERROR: {exc}"]
    log_markers = get_discord_log_markers_func()
    history_poll_task = getattr(bot, "_history_poll_task", None)
    history_poll_alive = isinstance(history_poll_task, DoneTaskLike) and not history_poll_task.done()
    intents = getattr(bot, "intents", None)
    intent_message_content = (
        isinstance(intents, MessageContentIntentLike) and intents.message_content
    )
    recent_events = get_recent_discord_hook_events_func()
    recent_user_events = get_recent_discord_hook_events_func(user_or_control_only=True)
    history_poll_primed_channels = _bot_int_set_attr(bot, "_history_poll_primed_channels")
    allowed_channel_ids = _bot_int_set_attr(bot, "allowed_channel_ids")
    allowed_user_ids = _bot_int_set_attr(bot, "allowed_user_ids")
    lines = [
        "Discord adapter diagnostics",
        f"channel_id: {channel_id or '-'}",
        f"mapped_thread_id: {target_thread_id or '-'}",
        f"project_channel: {project[1] if project else '-'}",
        f"message_content_enabled: {bool(getattr(bot, 'enable_prefix_commands', False))}",
        f"intent_message_content: {intent_message_content}",
        f"raw_debug_events: {bool(getattr(bot, '_enable_debug_events', False))}",
        f"qa_commands_enabled: {discord_qa_commands_enabled_func()}",
        f"history_poll_seconds: {getattr(bot, 'history_poll_seconds', '-')}",
        f"history_poll_alive: {history_poll_alive}",
        f"history_poll_last_at: {getattr(bot, '_history_poll_last_at', '-')}",
        f"history_poll_primed_channels: {len(history_poll_primed_channels)}",
        f"slash_sync_status: {getattr(bot, '_slash_sync_status', '-')}",
        f"slash_sync_last_at: {getattr(bot, '_slash_sync_last_at', '-')}",
        f"slash_sync_commands: {getattr(bot, '_slash_sync_commands', '-')}",
        f"allowed_channels: {format_discord_id_list(allowed_channel_ids)}",
        f"allowed_users: {format_discord_id_list(allowed_user_ids)}",
        f"startup_channel_id: {getattr(bot, 'startup_channel_id', None) or '-'}",
        f"empty_content_notice_channels: {empty_content_notice_count}",
        f"busy_choices_active: {active_busy_choices}",
        f"busy_choices_stale: {stale_busy_choices}",
        f"persistent_component_claims_active: {active_component_claims}",
        f"persistent_component_claims_stale: {stale_component_claims}",
        *build_discord_doctor_log_marker_lines(log_markers),
        "",
        *mirror_lines,
        "",
        "Mapped thread controls:",
        "busy steering: stale busy is warning-only; Steer now still sends to Codex.",
        "stop reply: use `Stop reply` or `!stop [ref]` to interrupt the mapped Codex reply.",
        "",
        "Expected live log sequence:",
        "message: socket_message_create -> message_received",
        "slash/button: socket_interaction_create -> interaction_received",
        "",
        "Recent user/control hook events:",
        *(recent_user_events or ["-"]),
        "",
        "Recent hook events:",
        *(recent_events or ["-"]),
    ]
    return "\n".join(lines)


async def build_discord_doctor_message_with_history(
    bot: DiagnosticBotLike,
    channel_id: int | None,
    channel: DiagnosticChannelLike | None,
    *,
    build_discord_doctor_message_func: DoctorMessageBuilder,
    build_discord_channel_history_lines_func: ChannelHistoryBuilder,
    build_discord_tracked_target_user_history_lines_func: TrackedTargetHistoryBuilder,
) -> str:
    history_lines = await build_discord_channel_history_lines_func(channel)
    tracked_history_lines = await build_discord_tracked_target_user_history_lines_func(bot)
    return (
        build_discord_doctor_message_func(bot, channel_id)
        + "\n\n"
        + "\n".join(history_lines)
        + "\n\n"
        + "\n".join(tracked_history_lines)
    )

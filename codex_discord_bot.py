"""Discord frontend harness for operating the local Codex app/web session."""
from __future__ import annotations
import asyncio  # noqa: ANYIO_OK
import hashlib
import sqlite3
import sys
import threading
import time
import traceback
from collections.abc import Awaitable, Callable, Iterable, Mapping
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Final, TYPE_CHECKING, TypeAlias, TypeGuard, cast

import discord

import codex_discord_app_server as discord_app_server
from codex_discord_app_server_thread_filter import filter_app_server_available_threads
import codex_app_server_transport as app_server_transport
import codex_desktop_bridge as bridge
import codex_discord_busy as discord_busy
import codex_discord_busy_choice_source_message as discord_busy_choice_source_message
from codex_discord_bridge_modules import (
    BRIDGE_APP_SERVER_DELIVERY,
    BRIDGE_ARCHIVE_TARGETS,
    BRIDGE_CONTEXT,
    BRIDGE_CONTEXT_REFRESH,
    BRIDGE_FINAL_ANSWER,
    BRIDGE_MIRROR_STATUS,
    BRIDGE_PENDING_INPUT_REPLY,
    BRIDGE_PROCESS_MODULE,
    BRIDGE_SELECTED_THREAD,
    BRIDGE_SESSION_MIRROR_EVENTS,
    BRIDGE_SESSION_STATE,
    BRIDGE_STALE_BUSY_STEER,
    BRIDGE_THREAD_STATE,
    BRIDGE_THREAD_TARGETS,
    BRIDGE_WHERE,
    get_mirror_scope_bridge_module,
    get_mirror_status_bridge_module,
    get_project_bridge_module,
    get_queue_target_bridge_module,
)
from codex_discord_steering_bridge_module import (
    get_steering_bridge_module,
    make_app_server_steering_result,
)
import codex_discord_bridge_process as bridge_process
import codex_discord_bot_shapes as discord_bot_shapes
import codex_discord_channel_gate as discord_channel_gate
import codex_discord_channel_cache as discord_channel_cache
import codex_discord_codex_app_menu as discord_codex_app_menu
import codex_discord_delivery as discord_delivery
import codex_discord_diagnostics as discord_diagnostics
import codex_discord_diagnostics_history as discord_diagnostics_history
import codex_discord_empty_content_notice as discord_empty_content_notice
import codex_discord_interaction_gate as discord_interaction_gate
import codex_discord_interaction_log as discord_interaction_log
import codex_discord_bot_compat_exports as discord_bot_compat_exports
import codex_discord_bot_core_wiring_runtime as discord_bot_core_wiring_runtime
import codex_discord_bot_runtime_wiring_runtime as discord_bot_runtime_wiring_runtime
import codex_discord_bot_state_wiring_runtime as discord_bot_state_wiring_runtime
import codex_discord_message_gate as discord_message_gate
import codex_discord_mirror_stale as discord_mirror_stale
import codex_discord_mirrored_busy_delegation as discord_mirrored_busy_delegation
import codex_discord_processed_message_runtime as discord_processed_message_runtime
import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_command_deps_factory as discord_prefix_command_deps_factory
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_dispatch_runtime as discord_prefix_dispatch_runtime
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_recorded_busy_transport as discord_recorded_busy_transport
import codex_discord_ready_cleanup as discord_ready_cleanup
import codex_discord_ready_runtime as discord_ready_runtime
import codex_discord_prompt_busy_result as discord_prompt_busy_result
import codex_discord_prompt_busy_retry as discord_prompt_busy_retry
import codex_discord_prompt_delivery_flow as discord_prompt_delivery_flow
import codex_discord_prompt_delivery_prepare as discord_prompt_delivery_prepare
import codex_discord_prompt_pending_delivery as discord_prompt_pending_delivery
import codex_discord_bot_skill_slash_runtime as discord_bot_skill_slash_runtime
import codex_discord_bot_socket_runtime as discord_bot_socket_runtime
import codex_discord_runner as discord_runner
import codex_discord_runner_runtime as discord_runner_runtime
import codex_discord_runtime as discord_runtime
import codex_discord_runtime_config as discord_runtime_config
import codex_discord_seen_cache as discord_seen_cache
import codex_discord_session_mirror_archive as discord_session_mirror_archive
import codex_discord_session_mirror_item_delivery as discord_session_mirror_item_delivery
import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_output_targets as discord_session_mirror_output_targets
import codex_discord_slash_prompt_commands as discord_slash_prompt_commands
import codex_discord_stale_busy_steer as discord_stale_busy_steer
import codex_discord_startup_probe as discord_startup_probe
import codex_discord_steering as discord_steering
import codex_discord_store as discord_store
from codex_discord_typing_pulse import (
    start_session_mirror_typing_pulse,
    stop_session_mirror_typing_pulse,
)
from codex_discord_components import (
    parse_busy_choice_custom_id,
    parse_input_choice_custom_id,
)
from codex_thread_models import ThreadInfo
from codex_discord_logging import (
    get_discord_log_markers,
    get_recent_discord_hook_events,
    log_line,
)
from codex_discord_text import (
    DISCORD_MAX_LEN,
    build_ask_start_message,
    build_startup_notice as build_startup_notice_text,
    build_steering_start_message as build_steering_start_message_text,
    env_flag,
    fit_single_message,
    format_discord_command_label,
    format_log_argv,
    format_log_text_len,
    parse_bounded_float_env,
)
from codex_discord_runner_queue import QueueJobValue
DiscordAllowedMessageChannel: TypeAlias = discord.abc.GuildChannel | discord.Thread | discord.abc.PrivateChannel | discord.abc.Messageable
SCRIPT_DIR = Path(__file__).resolve().parent
ENV_PATH = SCRIPT_DIR / ".env"
LOG_PATH = SCRIPT_DIR / "codex_discord_bot.log"
RUNTIME_LOCK_PATH = SCRIPT_DIR / ".codex_discord_bot.runtime.lock"
STOP_REQUEST_PATH = SCRIPT_DIR / ".codex_discord_bot.stop"
RUNTIME_MUTEX_NAME = "Local\\CodexDiscordBot_" + hashlib.sha256(str(SCRIPT_DIR).lower().encode("utf-8")).hexdigest()[:16]
MIRROR_DB_PATH = SCRIPT_DIR / "discord_mirror.sqlite"
ATTACHMENT_DOWNLOAD_DIR = SCRIPT_DIR / "discord_attachments"
THREAD_RUNNERS_LOCK = discord_runner.THREAD_RUNNERS_LOCK
THREAD_RUNNERS = discord_runner.THREAD_RUNNERS
RUNTIME_STATE = discord_runtime.DiscordRuntimeState()
SESSION_MIRROR_STATE = discord_session_mirror.SessionMirrorState()
UI_FALLBACK_LOCK = threading.Lock()
STREAM_REDIRECT_LOCK = threading.RLock()
INTERACTIVE_INPUT_TAG = "[choice_required]"
DISCORD_DELIVERY_EXCEPTIONS: Final[tuple[type[BaseException], ...]] = (discord.DiscordException, OSError, RuntimeError)
INTERACTIVE_APPROVAL_TAG = "[approval_required]"
INTERACTIVE_STATE_NONE = ""
INTERACTIVE_STATE_INPUT = "waiting-input"
INTERACTIVE_STATE_APPROVAL = "waiting-approval"
CODEX_PROJECTLESS_CHAT_KEY = "codex:chats"
BUSY_CHOICE_TTL_SECONDS = 1800
BUSY_CHOICE_COMPONENT_CLEANUP_HISTORY_LIMIT = 50
EMPTY_CONTENT_NOTICE_COOLDOWN_SECONDS = discord_empty_content_notice.EMPTY_CONTENT_NOTICE_COOLDOWN_SECONDS
EMPTY_CONTENT_NOTICE_LAST_SENT = discord_empty_content_notice.EMPTY_CONTENT_NOTICE_LAST_SENT
DISCORD_DELIVERY_STATE = discord_delivery.DiscordDeliveryState()
ACTIVE_DISCORD_DELIVERIES = DISCORD_DELIVERY_STATE.active_deliveries
discord_delivery_stopping = DISCORD_DELIVERY_STATE.stopping
DISCORD_RESTARTING_ERROR = discord_delivery.DISCORD_RESTARTING_ERROR
DISCORD_RESTARTING_NOTICE = discord_delivery.DISCORD_RESTARTING_NOTICE
DiscordDeliveryRejected = discord_delivery.DiscordDeliveryRejected
HISTORY_POLL_DEFAULT_SECONDS = 15.0
HISTORY_POLL_HISTORY_LIMIT = 10
PROCESSED_MESSAGE_ID_LIMIT = 2000
PROCESSED_MESSAGE_RETENTION_SECONDS = 86400.0
SOCKET_EVENT_LOG_ID_LIMIT = 2000
DISCORD_SEND_RETRY_DELAYS_SECONDS = discord_delivery.DISCORD_SEND_RETRY_DELAYS_SECONDS
DISCORD_CHUNK_MARKERS_ENABLED = discord_delivery.DISCORD_CHUNK_MARKERS_ENABLED
QUIET_PROGRESS_NOTICE_DELAY_SECONDS = -1.0
STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS = 25.0
STEERING_PENDING_WATCH_TIMEOUT_SECONDS = 600.0
STALE_BUSY_STEER_BLOCK_SECONDS = 600.0
ASK_BUSY_RETRY_ATTEMPTS = 0.0
ASK_BUSY_RETRY_DELAY_SECONDS = 8.0
SESSION_MIRROR_POLL_DEFAULT_SECONDS = 1.0
STARTUP_CHANNEL_PROBE_TIMEOUT_SECONDS = 5.0
STOP_MARKER_POLL_SECONDS = 1.0
STOP_MARKER_DRAIN_TIMEOUT_SECONDS = 20.0
STOP_MARKER_CLOSE_TIMEOUT_SECONDS = 20.0
SESSION_MIRROR_TARGET_LIMIT = 100
SESSION_MIRROR_EVENT_RETENTION_SECONDS = 7 * 86400.0
SESSION_MIRROR_RECENT_TEXT_TTL_SECONDS = 600.0
SESSION_MIRROR_ACTIVE_OUTPUT_TTL_SECONDS = 6 * 3600.0
SESSION_MIRROR_CURSOR_PRIME_PRESERVE_SECONDS = 10 * 60.0
SESSION_MIRROR_ARCHIVE_BACKLOG_MAX_EVENTS_DEFAULT = 200
CONTEXT_REFRESH_DEFAULT_LIMIT = 10
CONTEXT_REFRESH_MAX_LIMIT = 30
CONTEXT_REFRESH_MAX_CHARS = 16000
CONTEXT_REFRESH_ITEM_MAX_CHARS = 2000
DISCORD_ORIGIN_PROMPT_TTL_SECONDS = 900.0
RECENT_CODEX_APP_PROMPT_DEDUPE_SECONDS = 120.0
RECENT_CODEX_APP_PROMPT_SCAN_BYTES = 4 * 1024 * 1024
app_server_transport.DEFAULT_CLIENT.log_func = log_line
BusyChoiceAuthor = discord_bot_shapes.BusyChoiceAuthor
BusyChoiceSourceMessage = discord_bot_shapes.BusyChoiceSourceMessage
BusyChoiceStoreIdLike = discord_bot_shapes.BusyChoiceStoreIdLike
BusyChoiceStoreMessageLike = discord_bot_shapes.BusyChoiceStoreMessageLike
require_busy_choice_store_message = discord_bot_shapes.require_busy_choice_store_message
require_component_view_children = discord_bot_shapes.require_component_view_children
is_discord_button_item = discord_bot_shapes.is_discord_button_item
require_interaction_message = discord_bot_shapes.require_interaction_message
SessionMirrorOutputChannel = discord_bot_shapes.SessionMirrorOutputChannel
ThreadContextUsageLike = discord_bot_shapes.ThreadContextUsageLike
SkillSlashSourceAuthor = discord_bot_shapes.SkillSlashSourceAuthor
SlashAskSourceMessage = discord_bot_shapes.SlashAskSourceMessage
SeenCacheKey: TypeAlias = discord_seen_cache.SeenCacheKey
SeenCacheMap: TypeAlias = discord_seen_cache.SeenCacheMap
SocketEventPayload: TypeAlias = discord_bot_socket_runtime.SocketEventPayload
SocketEventData: TypeAlias = discord_bot_socket_runtime.SocketEventData
StaleMirrorThreadRow: TypeAlias = discord_mirror_stale.StaleMirrorThreadRow
StaleMirrorProjectRow: TypeAlias = discord_mirror_stale.StaleMirrorProjectRow
QueueRetractResult: TypeAlias = discord_runner_runtime.QueueRetractResult


DiscordMessageIdCarrier: TypeAlias = discord_processed_message_runtime.DiscordMessageIdCarrier
DiscordMessageIdInput: TypeAlias = discord_processed_message_runtime.DiscordMessageIdInput
SeenCacheOwner: TypeAlias = discord_seen_cache.SeenCacheOwner
ArchiveMirrorCleanupOwner: TypeAlias = discord_session_mirror_archive.ArchiveMirrorCleanupOwner
CompatModuleValue: TypeAlias = object
if TYPE_CHECKING:
    from codex_discord_bot_type_exports import *  # noqa: F403

    async def sync_codex_mirror(bot: CompatModuleValue, *, limit: int | None = None) -> str: ...
    def filter_mirrorable_threads(threads: Iterable[ThreadInfo]) -> list[ThreadInfo]: ...
    async def get_or_create_project_channel(guild: CompatModuleValue, category: CompatModuleValue, project_key: str, project_name: str) -> CompatModuleValue: ...
    async def get_or_create_thread_channel(codex_thread: ThreadInfo, project_key: str, project_channel: CompatModuleValue) -> CompatModuleValue: ...
    def upsert_mirror_project(project_key: str, project_name: str, channel_id: int) -> None: ...
    def upsert_mirror_thread(codex_thread: ThreadInfo, project_key: str, thread_name: str, project_channel_id: int, discord_thread_id: int) -> None: ...
CORE_WIRING_RUNTIME = discord_bot_core_wiring_runtime.BotCoreWiringRuntime(module=sys.modules[__name__])
CORE_WIRING_RUNTIME.install()


def init_mirror_db() -> None:
    discord_store.init_mirror_db(MIRROR_DB_PATH)


STATE_WIRING_RUNTIME = discord_bot_state_wiring_runtime.BotStateWiringRuntime(module=sys.modules[__name__])
STATE_WIRING_RUNTIME.install()
BOT_RUNTIME_WIRING_RUNTIME = discord_bot_runtime_wiring_runtime.BotRuntimeWiringRuntime(module=sys.modules[__name__])
BOT_RUNTIME_WIRING_RUNTIME.install()
COMPAT_EXPORTS_RUNTIME = discord_bot_compat_exports.BotCompatExportsRuntime(module=sys.modules[__name__])
COMPAT_EXPORTS_RUNTIME.install()
if __name__ == "__main__":
    raise SystemExit(cast(Callable[[], int], globals()["main"])())

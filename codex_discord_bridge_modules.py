from __future__ import annotations

from collections.abc import Mapping
from pathlib import Path
from typing import Final, Protocol, cast

import codex_app_server_transport_delivery as app_server_delivery
import codex_desktop_bridge as bridge
import codex_discord_archive_targets as discord_archive_targets
import codex_discord_bridge_process as bridge_process
import codex_discord_bridge_protocols as discord_bridge_protocols
import codex_discord_context as discord_context
import codex_discord_mirror_scope as discord_mirror_scope
import codex_discord_project_types as discord_project_types
import codex_discord_projects as discord_projects
import codex_discord_queue_targets as discord_queue_targets
import codex_discord_thread_state as discord_thread_state
import codex_discord_where as discord_where
from codex_thread_models import ThreadContextUsage, ThreadInfo


class BridgeModuleSource(Protocol):
    pass


def bridge_module_source() -> BridgeModuleSource:
    return bridge


CodexBridgeThreadLists = discord_bridge_protocols.CodexBridgeThreadLists
CodexBridgeMirrorStatusModule = discord_bridge_protocols.CodexBridgeMirrorStatusModule
CodexBridgePendingInputReplyModule = discord_bridge_protocols.CodexBridgePendingInputReplyModule
CodexBridgeSelectedThreadModule = discord_bridge_protocols.CodexBridgeSelectedThreadModule
CodexBridgeSessionStateModule = discord_bridge_protocols.CodexBridgeSessionStateModule
CodexBridgeFinalAnswerModule = discord_bridge_protocols.CodexBridgeFinalAnswerModule
CodexBridgeSessionMirrorEventModule = discord_bridge_protocols.CodexBridgeSessionMirrorEventModule
CodexBridgeContextRefreshModule = discord_bridge_protocols.CodexBridgeContextRefreshModule
CodexBridgeStaleBusySteerModule = discord_bridge_protocols.CodexBridgeStaleBusySteerModule
BRIDGE_MODULE = bridge_module_source()


BRIDGE_APP_SERVER_DELIVERY: Final[app_server_delivery.BridgeModule] = cast(
    app_server_delivery.BridgeModule,
    BRIDGE_MODULE,
)
BRIDGE_THREAD_TARGETS: Final[discord_queue_targets.QueueTargetBridge] = cast(
    discord_queue_targets.QueueTargetBridge,
    BRIDGE_MODULE,
)
BRIDGE_PROJECTS: Final[discord_projects.BridgeProjectModule] = cast(
    discord_projects.BridgeProjectModule,
    BRIDGE_MODULE,
)
BRIDGE_ARCHIVE_TARGETS: Final[discord_archive_targets.ArchiveTargetBridge] = cast(
    discord_archive_targets.ArchiveTargetBridge,
    BRIDGE_MODULE,
)
BRIDGE_THREAD_LISTS: Final[CodexBridgeThreadLists] = cast(
    CodexBridgeThreadLists,
    BRIDGE_MODULE,
)
BRIDGE_MIRROR_STATUS: Final[CodexBridgeMirrorStatusModule] = cast(
    CodexBridgeMirrorStatusModule,
    BRIDGE_MODULE,
)
BRIDGE_PENDING_INPUT_REPLY: Final[CodexBridgePendingInputReplyModule] = cast(
    CodexBridgePendingInputReplyModule,
    BRIDGE_MODULE,
)
BRIDGE_SELECTED_THREAD: Final[CodexBridgeSelectedThreadModule] = cast(
    CodexBridgeSelectedThreadModule,
    BRIDGE_MODULE,
)
BRIDGE_SESSION_STATE: Final[CodexBridgeSessionStateModule] = cast(
    CodexBridgeSessionStateModule,
    BRIDGE_MODULE,
)
BRIDGE_FINAL_ANSWER: Final[CodexBridgeFinalAnswerModule] = cast(
    CodexBridgeFinalAnswerModule,
    BRIDGE_MODULE,
)
BRIDGE_SESSION_MIRROR_EVENTS: Final[CodexBridgeSessionMirrorEventModule] = cast(
    CodexBridgeSessionMirrorEventModule,
    BRIDGE_MODULE,
)
BRIDGE_CONTEXT_REFRESH: Final[CodexBridgeContextRefreshModule] = cast(
    CodexBridgeContextRefreshModule,
    BRIDGE_MODULE,
)
BRIDGE_PROCESS_MODULE: Final[bridge_process.BridgeModule] = cast(
    bridge_process.BridgeModule,
    BRIDGE_MODULE,
)
BRIDGE_THREAD_STATE: Final[discord_thread_state.ThreadStateBridge] = cast(
    discord_thread_state.ThreadStateBridge,
    BRIDGE_MODULE,
)
BRIDGE_CONTEXT: Final[discord_context.DiscordContextBridge] = cast(
    discord_context.DiscordContextBridge,
    BRIDGE_MODULE,
)
BRIDGE_WHERE: Final[discord_where.WhereBridge] = cast(
    discord_where.WhereBridge,
    BRIDGE_MODULE,
)
BRIDGE_STALE_BUSY_STEER: Final[CodexBridgeStaleBusySteerModule] = cast(
    CodexBridgeStaleBusySteerModule,
    BRIDGE_MODULE,
)


class ProjectBridgeThreadTypeError(TypeError):
    def __init__(self, thread: discord_projects.ProjectThread) -> None:
        self.thread: discord_projects.ProjectThread = thread
        super().__init__(f"Expected bridge.ThreadInfo, got {type(thread).__name__}")


class CodexProjectBridge:
    @property
    def GLOBAL_STATE_PATH(self) -> Path:
        return BRIDGE_PROJECTS.GLOBAL_STATE_PATH

    def normalize_workspace_path(self, path: str) -> str:
        return BRIDGE_PROJECTS.normalize_workspace_path(path)

    def strip_windows_extended_prefix(self, path: str) -> str:
        return BRIDGE_PROJECTS.strip_windows_extended_prefix(path)

    def get_thread_workspace_name(self, thread: discord_projects.ProjectThread) -> str:
        if isinstance(thread, ThreadInfo):
            return BRIDGE_PROJECTS.get_thread_workspace_name(thread)
        raise ProjectBridgeThreadTypeError(thread)

    def load_json(self, path: Path) -> Mapping[str, discord_project_types.JsonValue]:
        return BRIDGE_PROJECTS.load_json(path)

    def choose_thread(self, thread_id: str, cwd: str | None) -> discord_projects.ProjectThread:
        return BRIDGE_PROJECTS.choose_thread(thread_id, cwd)


PROJECT_BRIDGE_MODULE: Final[discord_projects.BridgeProjectModule] = CodexProjectBridge()


def get_project_bridge_module() -> discord_projects.BridgeProjectModule:
    return PROJECT_BRIDGE_MODULE


class CodexMirrorScopeBridge:
    def load_user_root_threads(self) -> list[ThreadInfo]:
        return BRIDGE_THREAD_LISTS.load_user_root_threads()

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]:
        return BRIDGE_THREAD_LISTS.load_recent_threads(limit)

    def filter_thread_list_for_target(
        self,
        threads: list[ThreadInfo],
        target_thread_id: str,
        cwd: str | None,
    ) -> list[ThreadInfo]:
        _ = cwd
        return [thread for thread in threads if thread.id == target_thread_id]


MIRROR_SCOPE_BRIDGE_MODULE: Final[discord_mirror_scope.MirrorScopeBridge] = (
    CodexMirrorScopeBridge()
)


def get_mirror_scope_bridge_module() -> discord_mirror_scope.MirrorScopeBridge:
    return MIRROR_SCOPE_BRIDGE_MODULE


class CodexMirrorStatusBridge:
    def choose_thread(self, thread_id: str, cwd: str | None) -> ThreadInfo:
        return BRIDGE_MIRROR_STATUS.choose_thread(thread_id, cwd)

    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsage | None:
        return BRIDGE_MIRROR_STATUS.get_thread_context_usage(thread)

    def describe_thread_context_usage(self, context_usage: ThreadContextUsage) -> str:
        return BRIDGE_MIRROR_STATUS.describe_thread_context_usage(context_usage)

    def should_recommend_archive(
        self,
        thread: ThreadInfo,
        context_usage: ThreadContextUsage | None,
    ) -> bool:
        return BRIDGE_MIRROR_STATUS.should_recommend_archive(thread, context_usage)

    def get_thread_collaboration_mode(self, thread: ThreadInfo) -> str:
        return BRIDGE_MIRROR_STATUS.get_thread_collaboration_mode(thread)

    def get_thread_service_tier(self, thread: ThreadInfo) -> str:
        return BRIDGE_MIRROR_STATUS.get_thread_service_tier(thread)

    def format_thread_model_display(self, thread: ThreadInfo, mode: str, speed: str) -> str:
        return BRIDGE_MIRROR_STATUS.format_thread_model_display(thread, mode, speed)

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]:
        return BRIDGE_MIRROR_STATUS.load_recent_threads(limit)

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str:
        return BRIDGE_MIRROR_STATUS.get_thread_ui_name(thread_id, thread) or ""


MIRROR_STATUS_BRIDGE_MODULE: Final[CodexMirrorStatusBridge] = CodexMirrorStatusBridge()


def get_mirror_status_bridge_module() -> CodexMirrorStatusBridge:
    return MIRROR_STATUS_BRIDGE_MODULE


class CodexQueueTargetBridge:
    def resolve_thread_ref(self, ref: str) -> discord_queue_targets.QueueTargetThread:
        return BRIDGE_THREAD_TARGETS.resolve_thread_ref(ref)


QUEUE_TARGET_BRIDGE_MODULE: Final[discord_queue_targets.QueueTargetBridge] = (
    CodexQueueTargetBridge()
)


def get_queue_target_bridge_module() -> discord_queue_targets.QueueTargetBridge:
    return QUEUE_TARGET_BRIDGE_MODULE

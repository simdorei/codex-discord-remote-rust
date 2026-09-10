from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_app_server as discord_app_server
import codex_discord_archive_targets as discord_archive_targets
import codex_discord_bridge_process as bridge_process
import codex_discord_interactive as discord_interactive
import codex_discord_queue_targets as discord_queue_targets
import codex_discord_thread_state as discord_thread_state

GetThreadTargetsBridgeFunc = Callable[[], discord_queue_targets.QueueTargetBridge]
GetArchiveTargetsBridgeFunc = Callable[[], discord_archive_targets.ArchiveTargetBridge]
GetBridgeProcessModuleFunc = Callable[[], bridge_process.BridgeModule]
GetStreamRedirectLockFunc = Callable[[], bridge_process.RedirectLock]
GetThreadStateBridgeFunc = Callable[[], discord_thread_state.ThreadStateBridge]
AppServerTransportEnabledFunc = Callable[[], bool]
IsActiveSessionMirrorOutputTargetFunc = Callable[[str | None], bool]


class BridgeAppServerClient(discord_app_server.AppServerClient, Protocol):
    def cancel_pending_server_requests(self, thread_id: str) -> int: ...


GetAppServerClientFunc = Callable[[], BridgeAppServerClient | None]


class GetPendingInteractiveStateFunc(Protocol):
    def __call__(
        self,
        target_thread_id: str,
        *,
        client: discord_app_server.AppServerClient | None = None,
    ) -> str | None: ...


@dataclass(frozen=True, slots=True)
class BridgeCommandRuntime:
    get_thread_targets_bridge: GetThreadTargetsBridgeFunc
    get_mirrored_codex_thread_id: discord_queue_targets.GetMirroredCodexThreadIdFunc
    get_archive_targets_bridge: GetArchiveTargetsBridgeFunc
    get_bridge_process_module: GetBridgeProcessModuleFunc
    get_stream_redirect_lock: GetStreamRedirectLockFunc
    get_thread_state_bridge: GetThreadStateBridgeFunc
    resolve_thread_target_args: discord_archive_targets.ResolveThreadTargetArgsFunc
    resolve_selected_target_func: discord_thread_state.ResolveSelectedTargetFunc
    resolve_target_ref_func: discord_thread_state.ResolveTargetRefFunc
    get_selected_interactive_state_func: discord_thread_state.GetSelectedInteractiveStateFunc
    app_server_transport_enabled: AppServerTransportEnabledFunc
    get_pending_interactive_state: GetPendingInteractiveStateFunc
    get_app_server_client: GetAppServerClientFunc
    is_active_session_mirror_output_target: IsActiveSessionMirrorOutputTargetFunc
    log: discord_thread_state.LogFunc
    state_none: str
    state_input: str
    state_approval: str
    input_tag: str
    approval_tag: str

    def resolve_discord_thread_target_args(
        self,
        discord_channel_id: int | None,
        ref: str | None,
    ) -> list[str]:
        normalized = str(ref or "").strip()
        if normalized:
            thread = self.get_thread_targets_bridge().resolve_thread_ref(normalized)
            return ["--thread-id", thread.id]
        target_thread_id = self.get_mirrored_codex_thread_id(discord_channel_id)
        if target_thread_id:
            return ["--thread-id", target_thread_id]
        return []

    def resolve_discord_archive_target_args(
        self,
        discord_channel_id: int | None,
        ref: str | None,
    ) -> list[str]:
        return discord_archive_targets.resolve_discord_archive_target_args(
            discord_channel_id,
            ref,
            bridge_module=self.get_archive_targets_bridge(),
            resolve_thread_target_args_func=self.resolve_thread_target_args,
        )

    def run_bridge_command(self, argv: list[str]) -> tuple[int, str]:
        result = bridge_process.run_bridge_command(
            argv,
            bridge_module=self.get_bridge_process_module(),
            stream_redirect_lock=self.get_stream_redirect_lock(),
        )
        if result[0] != 0 or not argv or argv[0] != "stop":
            return result
        target_thread_id = ""
        for index, value in enumerate(argv[:-1]):
            if value == "--thread-id":
                target_thread_id = argv[index + 1].strip()
                break
        if not target_thread_id:
            target_thread_id = self.resolve_selected_target_func()[0] or ""
        client = self.get_app_server_client()
        if target_thread_id and client is not None:
            cancelled = client.cancel_pending_server_requests(target_thread_id)
            self.log(
                f"app_server_stop_cleanup target={target_thread_id} "
                + f"cancelled_requests={cancelled}"
            )
        return result

    def resolve_selected_target(self) -> tuple[str | None, str]:
        return discord_thread_state.resolve_selected_target(
            bridge_module=self.get_thread_state_bridge(),
            log_func=self.log,
        )

    def get_selected_interactive_state(self) -> tuple[str, str | None, str]:
        return discord_thread_state.get_selected_interactive_state(
            bridge_module=self.get_thread_state_bridge(),
            resolve_selected_target_func=self.resolve_selected_target_func,
            state_none=self.state_none,
            state_input=self.state_input,
            state_approval=self.state_approval,
            log_func=self.log,
        )

    def parse_interactive_notice(self, text: str) -> tuple[str, str, list[tuple[str, str]]]:
        return discord_interactive.parse_interactive_notice(
            text,
            state_none=self.state_none,
            state_input=self.state_input,
            state_approval=self.state_approval,
            input_tag=self.input_tag,
            approval_tag=self.approval_tag,
        )

    def resolve_target_ref(self, target_thread_id: str | None) -> tuple[str | None, str]:
        return discord_thread_state.resolve_target_ref(
            target_thread_id,
            bridge_module=self.get_thread_state_bridge(),
            resolve_selected_target_func=self.resolve_selected_target_func,
            log_func=self.log,
        )

    def get_interactive_state_for_thread(self, target_thread_id: str | None) -> tuple[str, str | None, str]:
        state, resolved_thread_id, target_ref = discord_thread_state.get_interactive_state_for_thread(
            target_thread_id,
            bridge_module=self.get_thread_state_bridge(),
            resolve_target_ref_func=self.resolve_target_ref_func,
            get_selected_interactive_state_func=self.get_selected_interactive_state_func,
            state_none=self.state_none,
            state_input=self.state_input,
            state_approval=self.state_approval,
            log_func=self.log,
        )
        if state == self.state_none and self.app_server_transport_enabled():
            app_server_target_id = resolved_thread_id or target_thread_id
            if app_server_target_id:
                pending_state = self.get_pending_interactive_state(
                    app_server_target_id,
                    client=self.get_app_server_client(),
                )
                if pending_state == "approval":
                    _resolved, resolved_ref = self.resolve_target_ref_func(app_server_target_id)
                    return self.state_approval, app_server_target_id, resolved_ref or target_ref
                if pending_state == "input":
                    _resolved, resolved_ref = self.resolve_target_ref_func(app_server_target_id)
                    return self.state_input, app_server_target_id, resolved_ref or target_ref
        return state, resolved_thread_id, target_ref

    def get_busy_state_for_thread(self, target_thread_id: str | None) -> tuple[str, str | None, str]:
        state, resolved_thread_id, target_ref = discord_thread_state.get_busy_state_for_thread(
            target_thread_id,
            bridge_module=self.get_thread_state_bridge(),
            resolve_target_ref_func=self.resolve_target_ref_func,
            log_func=self.log,
        )
        active_target_id = resolved_thread_id or target_thread_id
        if state == "idle" and self.is_active_session_mirror_output_target(active_target_id):
            return "busy", active_target_id, target_ref
        return state, resolved_thread_id, target_ref

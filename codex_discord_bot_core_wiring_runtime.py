from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import codex_app_server_transport as app_server_transport
import codex_discord_app_server as discord_app_server
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bridge_command_runtime as discord_bridge_command_runtime
import codex_discord_bridge_process as bridge_process
import codex_discord_interaction_errors as discord_interaction_errors
import codex_discord_runtime as discord_runtime
import codex_discord_runtime_config_accessors as discord_runtime_config_accessors
import codex_discord_runtime_state_bridge as discord_runtime_state_bridge
import codex_discord_session_mirror as discord_session_mirror
from codex_discord_bridge_modules import (
    BRIDGE_ARCHIVE_TARGETS,
    BRIDGE_PROCESS_MODULE,
    BRIDGE_THREAD_STATE,
    BRIDGE_THREAD_TARGETS,
)

ModuleValue: TypeAlias = object


class DiscordModuleLike(Protocol):
    errors: discord_interaction_errors.DiscordErrorsLike


@dataclass(frozen=True, slots=True)
class BotCoreWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_runtime_state_bridge()
        self._install_runtime_config_accessors()
        self._install_bridge_command_runtime()

    def _install_runtime_state_bridge(self) -> None:
        runtime_state_bridge = discord_runtime_state_bridge.RuntimeStateBridge(
            session_mirror_state=cast(
                discord_session_mirror.SessionMirrorState,
                getattr(self.module, "SESSION_MIRROR_STATE"),
            ),
            runtime_state=cast(
                discord_runtime.DiscordRuntimeState,
                getattr(self.module, "RUNTIME_STATE"),
            ),
            thread_runners=cast(
                Mapping[str, discord_runtime_state_bridge.RunnerRecord],
                getattr(self.module, "THREAD_RUNNERS"),
            ),
            thread_runners_lock=cast(asyncio.Lock, getattr(self.module, "THREAD_RUNNERS_LOCK")),
            active_output_ttl_seconds=cast(float, getattr(self.module, "SESSION_MIRROR_ACTIVE_OUTPUT_TTL_SECONDS")),
            runtime_mutex_name=cast(str, getattr(self.module, "RUNTIME_MUTEX_NAME")),
            get_runtime_lock_path=lambda: cast(Path, getattr(self.module, "RUNTIME_LOCK_PATH")),
            log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            on_expired_output_target=lambda target_thread_id, activated_at: cast(
                Callable[[str, float], None],
                getattr(
                    self.module,
                    "schedule_expired_session_mirror_output_target_release",
                ),
            )(target_thread_id, activated_at),
        )
        self._set("RUNTIME_STATE_BRIDGE", runtime_state_bridge)
        self._set("get_session_mirror_state", runtime_state_bridge.get_session_mirror_state)
        self._set("get_runtime_state", runtime_state_bridge.get_runtime_state)
        self._set("RUNNER_SNAPSHOT_LOCK", discord_runtime_state_bridge.RunnerSnapshotLock())
        self._set("snapshot_thread_runners", runtime_state_bridge.snapshot_thread_runners)
        self._set("is_thread_runner_busy", runtime_state_bridge.is_thread_runner_busy)
        self._set("claim_direct_ask_target", runtime_state_bridge.claim_direct_ask_target)
        self._set("release_direct_ask_target", runtime_state_bridge.release_direct_ask_target)
        self._set("get_ask_delivery_lock", runtime_state_bridge.get_ask_delivery_lock)
        self._set("mark_steering_handoff", runtime_state_bridge.mark_steering_handoff)
        self._set("had_steering_handoff_since", runtime_state_bridge.had_steering_handoff_since)
        self._set("register_discord_relay", runtime_state_bridge.register_discord_relay)
        self._set("is_discord_relay_stale", runtime_state_bridge.is_discord_relay_stale)
        self._set("acquire_runtime_instance_lock", runtime_state_bridge.acquire_runtime_instance_lock)
        self._set("remove_runtime_lock_for_current_process", runtime_state_bridge.remove_runtime_lock_for_current_process)
        self._set("exit_bot_process", runtime_state_bridge.exit_bot_process)
        discord_module = cast(DiscordModuleLike, getattr(self.module, "discord"))
        self._set(
            "is_interaction_already_acknowledged_error",
            discord_interaction_errors.make_interaction_already_acknowledged_error_checker(
                discord_module.errors,
            ),
        )

    def _install_runtime_config_accessors(self) -> None:
        accessors = discord_runtime_config_accessors.RuntimeConfigAccessors(
            steering_delivery_confirm_timeout_default=cast(
                float,
                getattr(self.module, "STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS"),
            ),
            steering_pending_watch_timeout_default=cast(
                float,
                getattr(self.module, "STEERING_PENDING_WATCH_TIMEOUT_SECONDS"),
            ),
            ask_busy_retry_delay_seconds_default=cast(
                float,
                getattr(self.module, "ASK_BUSY_RETRY_DELAY_SECONDS"),
            ),
            startup_channel_probe_timeout_default=cast(
                float,
                getattr(self.module, "STARTUP_CHANNEL_PROBE_TIMEOUT_SECONDS"),
            ),
        )
        self._set("RUNTIME_CONFIG_ACCESSORS", accessors)
        self._set("discord_session_mirror_enabled", accessors.discord_session_mirror_enabled)
        self._set("get_steering_delivery_confirm_timeout", accessors.get_steering_delivery_confirm_timeout)
        self._set("get_steering_pending_watch_timeout", accessors.get_steering_pending_watch_timeout)
        self._set("get_ask_busy_retry_delay_seconds", accessors.get_ask_busy_retry_delay_seconds)
        self._set("get_startup_channel_probe_timeout", accessors.get_startup_channel_probe_timeout)

    def _install_bridge_command_runtime(self) -> None:
        bridge_command_runtime = discord_bridge_command_runtime.BridgeCommandRuntime(
            get_thread_targets_bridge=lambda: BRIDGE_THREAD_TARGETS,
            get_mirrored_codex_thread_id=lambda channel_id: cast(
                Callable[[int | None], str | None],
                getattr(self.module, "get_mirrored_codex_thread_id"),
            )(channel_id),
            get_archive_targets_bridge=lambda: BRIDGE_ARCHIVE_TARGETS,
            get_bridge_process_module=lambda: BRIDGE_PROCESS_MODULE,
            get_stream_redirect_lock=lambda: cast(
                bridge_process.RedirectLock,
                getattr(self.module, "STREAM_REDIRECT_LOCK"),
            ),
            get_thread_state_bridge=lambda: BRIDGE_THREAD_STATE,
            resolve_thread_target_args=lambda channel_id, ref: cast(
                Callable[[int | None, str | None], list[str]],
                getattr(self.module, "resolve_discord_thread_target_args"),
            )(channel_id, ref),
            resolve_selected_target_func=lambda: cast(
                Callable[[], tuple[str | None, str]],
                getattr(self.module, "resolve_selected_target"),
            )(),
            resolve_target_ref_func=lambda target_thread_id: cast(
                Callable[[str | None], tuple[str | None, str]],
                getattr(self.module, "resolve_target_ref"),
            )(target_thread_id),
            get_selected_interactive_state_func=lambda: cast(
                Callable[[], tuple[str, str | None, str]],
                getattr(self.module, "get_selected_interactive_state"),
            )(),
            app_server_transport_enabled=lambda: cast(
                Callable[[], bool],
                getattr(self.module, "app_server_transport_enabled"),
            )(),
            get_pending_interactive_state=discord_app_server.get_pending_interactive_state,
            get_app_server_client=lambda: cast(discord_app_server.AppServerClient, app_server_transport.DEFAULT_CLIENT),
            is_active_session_mirror_output_target=lambda target_thread_id: cast(
                Callable[[str | None], bool],
                getattr(self.module, "is_active_session_mirror_output_target"),
            )(target_thread_id),
            log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            state_none=cast(str, getattr(self.module, "INTERACTIVE_STATE_NONE")),
            state_input=cast(str, getattr(self.module, "INTERACTIVE_STATE_INPUT")),
            state_approval=cast(str, getattr(self.module, "INTERACTIVE_STATE_APPROVAL")),
            input_tag=cast(str, getattr(self.module, "INTERACTIVE_INPUT_TAG")),
            approval_tag=cast(str, getattr(self.module, "INTERACTIVE_APPROVAL_TAG")),
        )
        self._set("BRIDGE_COMMAND_RUNTIME", bridge_command_runtime)
        self._set("resolve_discord_thread_target_args", bridge_command_runtime.resolve_discord_thread_target_args)
        self._set("resolve_discord_archive_target_args", bridge_command_runtime.resolve_discord_archive_target_args)
        self._set("run_bridge_command", bridge_command_runtime.run_bridge_command)
        self._set("resolve_selected_target", bridge_command_runtime.resolve_selected_target)
        self._set("get_selected_interactive_state", bridge_command_runtime.get_selected_interactive_state)
        self._set("parse_interactive_notice", bridge_command_runtime.parse_interactive_notice)

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )

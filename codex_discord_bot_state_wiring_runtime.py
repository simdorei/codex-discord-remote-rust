from __future__ import annotations

import sqlite3
import time
import traceback
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import TypeAlias, cast

import codex_app_server_transport as app_server_transport
import codex_discord_busy_component_runtime as discord_busy_component_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_interaction_channel_runtime as discord_interaction_channel_runtime
import codex_discord_processed_message_runtime as discord_processed_message_runtime
import codex_discord_project_types as discord_project_types
import codex_discord_project_runtime as discord_project_runtime
import codex_discord_ready_runtime as discord_ready_runtime
import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_cursor as discord_session_mirror_cursor
import codex_discord_session_mirror_release_runtime as discord_session_mirror_release_runtime
import codex_discord_session_mirror_state_runtime as discord_session_mirror_state_runtime
import codex_discord_stale_busy_components as discord_stale_busy_components
import codex_discord_store as discord_store
from codex_discord_bridge_modules import BRIDGE_THREAD_STATE, get_project_bridge_module

ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotStateWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_session_mirror_state()
        self._install_busy_components()
        self._install_startup_probe_targets()
        self._install_project_runtime()
        self._install_interaction_channel_runtime()
        self._install_processed_message_runtime()

    def _install_session_mirror_state(self) -> None:
        release_runtime = discord_session_mirror_release_runtime.SessionMirrorReleaseRuntime(
            discord_session_mirror_release_runtime.SessionMirrorReleaseRuntimeDeps(
                transport_enabled=lambda: cast(
                    Callable[[], bool],
                    getattr(self.module, "app_server_transport_enabled"),
                )(),
                client=app_server_transport.DEFAULT_CLIENT,
                get_output_target_activation=self._get_output_target_activation,
                deactivate_output_target_if_unchanged=(
                    self._deactivate_session_mirror_output_target_if_unchanged
                ),
                clear_expiring_output_target=self._clear_expiring_output_target,
                log=self._log,
            )
        )
        runtime = discord_session_mirror_state_runtime.SessionMirrorStateRuntime(
            get_db_path=self._get_db_path,
            get_session_mirror_state=self._get_session_mirror_state,
            session_mirror_enabled=self._session_mirror_enabled,
            choose_thread=self._choose_thread,
            get_or_init_cursor=self._get_or_init_cursor,
            update_cursor=self._update_cursor,
            is_active_output_target=self._is_active_output_target,
            time_now=time.time,
            preserve_seconds=cast(float, getattr(self.module, "SESSION_MIRROR_CURSOR_PRIME_PRESERVE_SECONDS")),
            active_ttl_seconds=cast(float, getattr(self.module, "SESSION_MIRROR_ACTIVE_OUTPUT_TTL_SECONDS")),
            on_expired_output_target=release_runtime.schedule_expired_output_target,
            exception_types=(OSError, RuntimeError, sqlite3.Error),
            format_exception=traceback.format_exc,
            log=self._log,
        )
        self._set("SESSION_MIRROR_STATE_RUNTIME", runtime)
        self._set("SESSION_MIRROR_RELEASE_RUNTIME", release_runtime)
        self._set(
            "schedule_expired_session_mirror_output_target_release",
            release_runtime.schedule_expired_output_target,
        )
        self._set("get_or_init_session_mirror_cursor", runtime.get_or_init_session_mirror_cursor)
        self._set("update_session_mirror_cursor", runtime.update_session_mirror_cursor)
        self._set("prime_session_mirror_cursor_for_target", runtime.prime_session_mirror_cursor_for_target)
        self._set("activate_session_mirror_output_target", runtime.activate_session_mirror_output_target)
        self._set("activate_pending_session_mirror_output_target", runtime.activate_pending_session_mirror_output_target)
        self._set("deactivate_session_mirror_output_target", self._deactivate_session_mirror_output_target)
        self._set(
            "get_session_mirror_output_target_activation",
            runtime.get_session_mirror_output_target_activation,
        )
        self._set(
            "deactivate_session_mirror_output_target_if_unchanged",
            self._deactivate_session_mirror_output_target_if_unchanged,
        )
        self._set(
            "release_session_mirror_output_target",
            release_runtime.release_output_target,
        )
        self._set("is_active_session_mirror_output_target", runtime.is_active_session_mirror_output_target)
        self._set("is_pending_session_mirror_cursor_target", runtime.is_pending_session_mirror_cursor_target)
        self._set("clear_pending_session_mirror_cursor_target", runtime.clear_pending_session_mirror_cursor_target)
        self._set("session_mirror_rollout_path_missing", runtime.session_mirror_rollout_path_missing)
        self._set("claim_session_mirror_event", runtime.claim_session_mirror_event)
        self._set("has_session_mirror_event", runtime.has_session_mirror_event)

    def _install_busy_components(self) -> None:
        runtime = discord_busy_component_runtime.BusyComponentRuntime(
            get_db_path=self._get_db_path,
            get_busy_choice_record_func=self._get_busy_choice_record,
            cleanup_history_limit=cast(int, getattr(self.module, "BUSY_CHOICE_COMPONENT_CLEANUP_HISTORY_LIMIT")),
            log=self._log,
        )
        self._set("BUSY_COMPONENT_RUNTIME", runtime)
        self._set("cleanup_expired_busy_choices", runtime.cleanup_expired_busy_choices)
        self._set("cleanup_expired_persistent_component_claims", runtime.cleanup_expired_persistent_component_claims)
        self._set("get_busy_choice_counts", runtime.get_busy_choice_counts)
        self._set("get_persistent_component_claim_counts", runtime.get_persistent_component_claim_counts)
        self._set("get_busy_choice_record", runtime.get_busy_choice_record)
        self._set("claim_busy_choice_record", runtime.claim_busy_choice_record)
        self._set("claim_persistent_component_interaction", runtime.claim_persistent_component_interaction)
        self._set("clear_stale_busy_choice_message_components", runtime.clear_stale_busy_choice_message_components)
        self._set("cleanup_stale_busy_choice_components_in_channel", runtime.cleanup_stale_busy_choice_components_in_channel)

    def _install_startup_probe_targets(self) -> None:
        runtime = discord_ready_runtime.StartupProbeTargetRuntime(get_db_path=self._get_db_path)
        self._set("STARTUP_PROBE_TARGET_RUNTIME", runtime)
        self._set("get_startup_probe_targets", runtime.get_startup_probe_targets)

    def _install_project_runtime(self) -> None:
        runtime = discord_project_runtime.ProjectRuntime(
            get_db_path=self._get_db_path,
            get_project_bridge_module=get_project_bridge_module,
            projectless_chat_key=cast(str, getattr(self.module, "CODEX_PROJECTLESS_CHAT_KEY")),
            init_mirror_db_func=lambda: discord_store.init_mirror_db(self._get_db_path()),
            get_mirrored_codex_thread_id_func=self._get_mirrored_codex_thread_id,
            get_thread_cwd_func=self._get_thread_cwd,
            get_mirror_project_for_channel_func=self._get_mirror_project_for_channel,
            project_keys_match_func=self._project_keys_match,
        )
        self._set("PROJECT_RUNTIME", runtime)
        self._set("get_project_key", runtime.get_project_key)
        self._set("normalize_project_key", runtime.normalize_project_key)
        self._set("get_project_name", runtime.get_project_name)
        self._set("filter_mirrorable_threads", runtime.filter_mirrorable_threads)
        self._set("get_mirrored_codex_thread_id", runtime.get_mirrored_codex_thread_id)
        self._set("persist_inbound_mirror_thread_channel", runtime.persist_inbound_mirror_thread_channel)
        self._set("describe_mirrored_project_channel", runtime.describe_mirrored_project_channel)
        self._set("get_mirror_project_for_channel", runtime.get_mirror_project_for_channel)
        self._set("get_thread_cwd", runtime.get_thread_cwd)
        self._set("resolve_discord_new_thread_cwd", runtime.resolve_discord_new_thread_cwd)
        self._set("project_keys_match", runtime.project_keys_match)
        self._set("resolve_discord_new_thread_project_channel_id", runtime.resolve_discord_new_thread_project_channel_id)
        self._set("is_mirrored_channel_id", runtime.is_mirrored_channel_id)

    def _install_interaction_channel_runtime(self) -> None:
        runtime = discord_interaction_channel_runtime.InteractionChannelRuntime(
            is_mirrored_channel_id_func=self._is_mirrored_channel_id,
        )
        self._set("INTERACTION_CHANNEL_RUNTIME", runtime)
        self._set("get_interaction_gate_command_name", runtime.get_interaction_gate_command_name)
        self._set("coerce_interaction_channel_id", runtime.coerce_interaction_channel_id)
        self._set("coerce_delivery_state_discord_id", runtime.coerce_delivery_state_discord_id)
        self._set("is_mirrored_interaction_channel_id", runtime.is_mirrored_interaction_channel_id)

    def _install_processed_message_runtime(self) -> None:
        runtime = discord_processed_message_runtime.ProcessedMessageRuntime(
            get_db_path=self._get_db_path,
            processed_message_id_limit=cast(int, getattr(self.module, "PROCESSED_MESSAGE_ID_LIMIT")),
            get_message_id_func=self._get_discord_message_id,
            claim_persistent_message_id_func=self._claim_persistent_discord_message_id,
            log=self._log,
        )
        self._set("PROCESSED_MESSAGE_RUNTIME", runtime)
        self._set("get_discord_message_id", runtime.get_discord_message_id)
        self._set("claim_persistent_discord_message_id", runtime.claim_persistent_discord_message_id)
        self._set("claim_discord_message", runtime.claim_discord_message)
        self._set("mark_discord_message_processed", runtime.mark_discord_message_processed)

    def _get_db_path(self) -> Path:
        return cast(Path, getattr(self.module, "MIRROR_DB_PATH"))

    def _deactivate_session_mirror_output_target(self, target_thread_id: str | None) -> None:
        cast(Callable[[str | None], None], getattr(self.module, "stop_session_mirror_typing_pulse"))(
            target_thread_id,
        )
        runtime = cast(
            discord_session_mirror_state_runtime.SessionMirrorStateRuntime,
            getattr(self.module, "SESSION_MIRROR_STATE_RUNTIME"),
        )
        runtime.deactivate_session_mirror_output_target(target_thread_id)

    def _deactivate_session_mirror_output_target_if_unchanged(
        self,
        target_thread_id: str | None,
        expected_activation: float | None,
    ) -> bool:
        runtime = cast(
            discord_session_mirror_state_runtime.SessionMirrorStateRuntime,
            getattr(self.module, "SESSION_MIRROR_STATE_RUNTIME"),
        )
        deactivated = runtime.deactivate_session_mirror_output_target_if_unchanged(
            target_thread_id,
            expected_activation,
        )
        if deactivated:
            stop_typing = getattr(self.module, "stop_session_mirror_typing_pulse", None)
            if callable(stop_typing):
                cast(Callable[[str | None], None], stop_typing)(target_thread_id)
        return deactivated

    def _get_output_target_activation(
        self,
        target_thread_id: str | None,
    ) -> float | None:
        runtime = cast(
            discord_session_mirror_state_runtime.SessionMirrorStateRuntime,
            getattr(self.module, "SESSION_MIRROR_STATE_RUNTIME"),
        )
        return runtime.get_session_mirror_output_target_activation(target_thread_id)

    def _clear_expiring_output_target(
        self,
        target_thread_id: str,
        expected_activation: float,
    ) -> None:
        runtime = cast(
            discord_session_mirror_state_runtime.SessionMirrorStateRuntime,
            getattr(self.module, "SESSION_MIRROR_STATE_RUNTIME"),
        )
        runtime.clear_expiring_session_mirror_output_target(
            target_thread_id,
            expected_activation,
        )

    def _get_session_mirror_state(self) -> discord_session_mirror.SessionMirrorState:
        return cast(
            Callable[[], discord_session_mirror.SessionMirrorState],
            getattr(self.module, "get_session_mirror_state"),
        )()

    def _session_mirror_enabled(self) -> bool:
        return cast(Callable[[], bool], getattr(self.module, "discord_session_mirror_enabled"))()

    def _choose_thread(
        self,
        ref: str,
        fallback: str | None = None,
    ) -> discord_session_mirror_cursor.SessionMirrorCursorThread:
        return cast(
            discord_session_mirror_cursor.SessionMirrorCursorThread,
            BRIDGE_THREAD_STATE.choose_thread(ref, fallback),
        )

    def _get_or_init_cursor(self, codex_thread_id: str, rollout_path: str, initial_cursor: int) -> int:
        return cast(
            discord_session_mirror_state_runtime.SessionMirrorCursorGetter,
            getattr(self.module, "get_or_init_session_mirror_cursor"),
        )(codex_thread_id, rollout_path, initial_cursor)

    def _update_cursor(self, codex_thread_id: str, rollout_path: str, cursor: int) -> None:
        cast(
            discord_session_mirror_state_runtime.SessionMirrorCursorUpdater,
            getattr(self.module, "update_session_mirror_cursor"),
        )(codex_thread_id, rollout_path, cursor)

    def _is_active_output_target(self, target_thread_id: str) -> bool:
        return cast(
            discord_session_mirror_state_runtime.OutputTargetPredicate,
            getattr(self.module, "is_active_session_mirror_output_target"),
        )(target_thread_id)

    def _get_busy_choice_record(self, choice_id: str) -> discord_stale_busy_components.BusyChoiceRecord | None:
        return cast(
            discord_busy_component_runtime.BusyChoiceRecordGetter,
            getattr(self.module, "get_busy_choice_record"),
        )(choice_id)

    def _get_mirrored_codex_thread_id(self, channel_id: int | None) -> str | None:
        return cast(
            discord_project_types.GetMirroredCodexThreadId,
            getattr(self.module, "get_mirrored_codex_thread_id"),
        )(channel_id)

    def _get_thread_cwd(self, thread_id: str | None) -> str | None:
        return cast(discord_project_types.GetThreadCwd, getattr(self.module, "get_thread_cwd"))(thread_id)

    def _get_mirror_project_for_channel(self, channel_id: int | None) -> tuple[str, str] | None:
        return cast(
            discord_project_types.GetMirrorProjectForChannel,
            getattr(self.module, "get_mirror_project_for_channel"),
        )(channel_id)

    def _project_keys_match(self, left: str | None, right: str | None) -> bool:
        return cast(discord_project_types.ProjectKeysMatch, getattr(self.module, "project_keys_match"))(left, right)

    def _is_mirrored_channel_id(self, channel_id: int | None) -> bool:
        return cast(Callable[[int | None], bool], getattr(self.module, "is_mirrored_channel_id"))(channel_id)

    def _get_discord_message_id(self, message: discord_processed_message_runtime.DiscordMessageIdInput) -> int | None:
        return cast(
            discord_processed_message_runtime.GetMessageIdFunc,
            getattr(self.module, "get_discord_message_id"),
        )(message)

    def _claim_persistent_discord_message_id(self, message_id: int) -> bool:
        return cast(
            discord_processed_message_runtime.ClaimPersistentMessageIdFunc,
            getattr(self.module, "claim_persistent_discord_message_id"),
        )(message_id)

    def _log(self, message: str) -> None:
        cast(Callable[[str], None], getattr(self.module, "log_line"))(message)

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )

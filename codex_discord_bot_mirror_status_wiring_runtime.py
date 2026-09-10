from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_bot_context_adapter_runtime as discord_bot_context_adapter_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_diagnostics_adapter_runtime as discord_bot_diagnostics_adapter_runtime
import codex_discord_bot_mirror_adapter_runtime as discord_bot_mirror_adapter_runtime
import codex_discord_bot_stale_busy_adapter_runtime as discord_bot_stale_busy_adapter_runtime
import codex_discord_mirror_channel_runtime as discord_mirror_channel_runtime
import codex_discord_mirror_single_thread as discord_mirror_single_thread
import codex_discord_mirror_stale as discord_mirror_stale
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotMirrorStatusWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_mirror_channels()
        self._install_mirror_status()
        self._install_context_and_diagnostics()

    def _install_mirror_channels(self) -> None:
        self._set("MirrorGuildUnavailableError", discord_mirror_channel_runtime.MirrorGuildUnavailableError)
        self._set(
            "PreferredMirrorProjectChannelUnavailableError",
            discord_mirror_single_thread.PreferredMirrorProjectChannelUnavailableError,
        )
        self._set("PreferredMirrorProjectChannelTypeError", discord_mirror_single_thread.PreferredMirrorProjectChannelTypeError)
        runtime = discord_mirror_channel_runtime.MirrorChannelRuntime(
            get_db_path=lambda: cast(Path, getattr(self.module, "MIRROR_DB_PATH")),
            normalize_project_key=cast(discord_mirror_channel_runtime.NormalizeProjectKeyFunc, getattr(self.module, "normalize_project_key")),
            project_keys_match=cast(discord_mirror_channel_runtime.ProjectKeysMatchFunc, getattr(self.module, "project_keys_match")),
            get_thread_ui_name=self._get_thread_ui_name,
            log=cast(Callable[[str], None], getattr(self.module, "log_line")),
        )
        self._set("MIRROR_CHANNEL_RUNTIME", runtime)
        self._set("get_mirror_guild", cast(object, runtime.get_mirror_guild))
        self._set("get_or_create_mirror_category", cast(object, runtime.get_or_create_mirror_category))
        self._set("upsert_mirror_project", runtime.upsert_mirror_project)
        self._set("upsert_mirror_thread", runtime.upsert_mirror_thread)
        self._set("ensure_mirror_project_channel", cast(object, runtime.ensure_mirror_project_channel))
        self._set("get_or_create_project_channel", cast(object, runtime.get_or_create_project_channel))
        self._set("get_or_create_thread_channel", cast(object, runtime.get_or_create_thread_channel))
        self._set("delete_stale_discord_threads", cast(object, discord_mirror_stale.delete_stale_discord_threads))
        self._set("delete_stale_project_channels", cast(object, discord_mirror_stale.delete_stale_project_channels))

    def _install_mirror_status(self) -> None:
        adapter_runtime = discord_bot_mirror_adapter_runtime.BotMirrorAdapterRuntime(module=self.module)
        mirror_runtime = adapter_runtime.make_mirror_runtime()
        self._set("MIRROR_ADAPTER_RUNTIME", adapter_runtime)
        self._set("MIRROR_RUNTIME", mirror_runtime)
        self._set("load_mirror_scope_threads", mirror_runtime.load_mirror_scope_threads)
        self._set("filter_threads_for_discord_channel", mirror_runtime.filter_threads_for_discord_channel)
        self._set("sync_codex_mirror", mirror_runtime.sync_codex_mirror)
        self._set("refresh_codex_bridge_session_state", mirror_runtime.refresh_codex_bridge_session_state)
        self._set("refresh_discord_bridge_session", mirror_runtime.refresh_discord_bridge_session)
        self._set("mirror_single_codex_thread", mirror_runtime.mirror_single_codex_thread)
        self._set("build_mirror_list", mirror_runtime.build_mirror_list)
        self._set("build_mirror_list_for_prefix", mirror_runtime.build_mirror_list_for_prefix)
        self._set("build_mirror_check", mirror_runtime.build_mirror_check)
        self._set("build_mirror_check_for_prefix", mirror_runtime.build_mirror_check_for_prefix)

    def _install_context_and_diagnostics(self) -> None:
        context_runtime = discord_bot_context_adapter_runtime.BotContextAdapterRuntime(module=self.module)
        self._set("CONTEXT_ADAPTER_RUNTIME", context_runtime)
        self._set("format_context_usage_line", context_runtime.format_context_usage_line)
        self._set("build_context_warning", context_runtime.build_context_warning)
        self._set("build_context_message", context_runtime.build_context_message)
        self._set("build_context_refresh_message", context_runtime.build_context_refresh_message)
        stale_busy_runtime = discord_bot_stale_busy_adapter_runtime.BotStaleBusyAdapterRuntime(module=self.module)
        self._set("STALE_BUSY_ADAPTER_RUNTIME", stale_busy_runtime)
        self._set("get_stale_busy_steer_block_info", stale_busy_runtime.get_stale_busy_steer_block_info)
        self._set("send_stale_busy_steer_block_message", stale_busy_runtime.send_stale_busy_steer_block_message)
        self._set("build_stale_busy_steer_block_message", stale_busy_runtime.build_stale_busy_steer_block_message)
        self._set("format_weekly_usage_percent", context_runtime.format_weekly_usage_percent)
        self._set("build_weekly_usage_message", context_runtime.build_weekly_usage_message)
        self._set("build_where_message", context_runtime.build_where_message)
        diagnostics_runtime = discord_bot_diagnostics_adapter_runtime.BotDiagnosticsAdapterRuntime(module=self.module)
        self._set("DIAGNOSTICS_ADAPTER_RUNTIME", diagnostics_runtime)
        self._set("build_discord_doctor_message", diagnostics_runtime.build_discord_doctor_message)
        self._set("build_discord_channel_history_lines", diagnostics_runtime.build_discord_channel_history_lines)
        self._set("build_discord_tracked_target_user_history_lines", diagnostics_runtime.build_discord_tracked_target_user_history_lines)
        self._set("build_discord_doctor_message_with_history", diagnostics_runtime.build_discord_doctor_message_with_history)

    def _get_thread_ui_name(self) -> discord_mirror_channel_runtime.GetThreadUiNameFunc:
        bridge = cast(object, getattr(self.module, "BRIDGE_MIRROR_STATUS"))
        return cast(discord_mirror_channel_runtime.GetThreadUiNameFunc, getattr(bridge, "get_thread_ui_name"))

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )

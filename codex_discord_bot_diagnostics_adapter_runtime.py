from __future__ import annotations

from collections.abc import Callable, Sized
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast

import codex_discord_diagnostics as discord_diagnostics
import codex_discord_diagnostics_history as discord_diagnostics_history


class StartupProbeTargetsGetter(Protocol):
    def __call__(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 30,
    ) -> list[tuple[str, int]]: ...


class DiagnosticsRuntimeConfig(Protocol):
    def discord_qa_commands_enabled(self) -> bool: ...


@dataclass(frozen=True, slots=True)
class BotDiagnosticsAdapterRuntime:
    module: ModuleType

    def build_discord_doctor_message(
        self,
        bot: discord_diagnostics.DiagnosticBotLike,
        channel_id: int | None,
    ) -> str:
        empty_content_notice_last_sent = cast(Sized, getattr(self.module, "EMPTY_CONTENT_NOTICE_LAST_SENT"))
        runtime_config = cast(DiagnosticsRuntimeConfig, getattr(self.module, "discord_runtime_config"))
        return discord_diagnostics.build_discord_doctor_message(
            bot,
            channel_id,
            empty_content_notice_count=len(empty_content_notice_last_sent),
            get_mirrored_codex_thread_id_func=cast(
                discord_diagnostics.MirroredThreadIdGetter,
                getattr(self.module, "get_mirrored_codex_thread_id"),
            ),
            get_mirror_project_for_channel_func=cast(
                discord_diagnostics.MirrorProjectGetter,
                getattr(self.module, "get_mirror_project_for_channel"),
            ),
            get_busy_choice_counts_func=cast(discord_diagnostics.CountGetter, getattr(self.module, "get_busy_choice_counts")),
            get_persistent_component_claim_counts_func=cast(
                discord_diagnostics.CountGetter,
                getattr(self.module, "get_persistent_component_claim_counts"),
            ),
            build_mirror_check_func=cast(discord_diagnostics.MirrorCheckBuilder, getattr(self.module, "build_mirror_check")),
            get_discord_log_markers_func=cast(
                discord_diagnostics.DiscordLogMarkersGetter,
                getattr(self.module, "get_discord_log_markers"),
            ),
            get_recent_discord_hook_events_func=cast(
                discord_diagnostics.RecentDiscordHookEventsGetter,
                getattr(self.module, "get_recent_discord_hook_events"),
            ),
            discord_qa_commands_enabled_func=runtime_config.discord_qa_commands_enabled,
        )

    async def build_discord_channel_history_lines(
        self,
        channel: discord_diagnostics_history.DiscordChannelLike | None,
    ) -> list[str]:
        return await discord_diagnostics.build_discord_channel_history_lines(
            channel,
            format_log_text_len_func=cast(Callable[[str | None], int], getattr(self.module, "format_log_text_len")),
        )

    async def build_discord_tracked_target_user_history_lines(
        self,
        bot: discord_diagnostics.DiagnosticBotLike,
    ) -> list[str]:
        return await discord_diagnostics.build_discord_tracked_target_user_history_lines(
            bot,
            get_startup_probe_targets_func=self._get_startup_probe_targets,
            format_log_text_len_func=cast(Callable[[str | None], int], getattr(self.module, "format_log_text_len")),
        )

    async def build_discord_doctor_message_with_history(
        self,
        bot: discord_diagnostics.DiagnosticBotLike,
        channel_id: int | None,
        channel: discord_diagnostics.DiagnosticChannelLike | None,
    ) -> str:
        return await discord_diagnostics.build_discord_doctor_message_with_history(
            bot,
            channel_id,
            channel,
            build_discord_doctor_message_func=cast(
                discord_diagnostics.DoctorMessageBuilder,
                getattr(self.module, "build_discord_doctor_message"),
            ),
            build_discord_channel_history_lines_func=cast(
                discord_diagnostics.ChannelHistoryBuilder,
                getattr(self.module, "build_discord_channel_history_lines"),
            ),
            build_discord_tracked_target_user_history_lines_func=cast(
                discord_diagnostics.TrackedTargetHistoryBuilder,
                getattr(self.module, "build_discord_tracked_target_user_history_lines"),
            ),
        )

    def _get_startup_probe_targets(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 30,
    ) -> list[tuple[str, int]]:
        get_startup_probe_targets = cast(StartupProbeTargetsGetter, getattr(self.module, "get_startup_probe_targets"))
        return get_startup_probe_targets(allowed_channel_ids, startup_channel_id, limit=limit)

from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, Protocol, TypeVar, cast

import codex_discord_plain_ask as discord_plain_ask
import codex_discord_ready_cleanup as discord_ready_cleanup
import codex_discord_startup_probe as discord_startup_probe
import codex_discord_store as discord_store

ChannelT = TypeVar("ChannelT")
GetDbPathFunc = Callable[[], Path]


@dataclass(frozen=True, slots=True)
class StartupProbeTargetRuntime:
    get_db_path: GetDbPathFunc

    def get_startup_probe_targets(
        self,
        allowed_channel_ids: set[int],
        startup_channel_id: int | None,
        *,
        limit: int = 30,
    ) -> list[tuple[str, int]]:
        return discord_store.get_startup_probe_targets(
            self.get_db_path(),
            allowed_channel_ids,
            startup_channel_id,
            limit=limit,
        )


class ReadyBot(Protocol[ChannelT]):
    allowed_channel_ids: set[int]
    startup_channel_id: int | None

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[ChannelT | None, str]: ...
    def fetch_channel(self, channel_id: int) -> Awaitable[ChannelT]: ...
    def is_allowed_message_channel(self, channel: ChannelT) -> bool: ...


@dataclass(frozen=True, slots=True)
class ReadyRuntime(Generic[ChannelT]):
    delivery_exceptions: tuple[type[BaseException], ...]
    is_messageable: Callable[[ChannelT], bool]
    get_startup_probe_targets: discord_startup_probe.ProbeTargetsGetter
    get_startup_probe_timeout: discord_startup_probe.ProbeTimeoutGetter
    format_traceback: discord_startup_probe.TracebackFormatter
    build_prompt_with_discord_attachments: discord_plain_ask.BuildPromptWithAttachmentsFunc[
        discord_plain_ask.PlainAskPreparedMessage
    ]
    send_chunks: discord_plain_ask.SendChunksFunc[int]
    cleanup_expired_busy_choices: discord_ready_cleanup.ReadyCleanup
    cleanup_expired_persistent_component_claims: discord_ready_cleanup.ReadyCleanup
    cleanup_processed_messages: discord_ready_cleanup.ReadyCleanup
    cleanup_session_mirror_events: discord_ready_cleanup.ReadyCleanup
    cleanup_stale_busy_choice_components_in_channel: discord_ready_cleanup.StaleBusyChoiceChannelCleanup[ChannelT]
    log: Callable[[str], None]

    def make_plain_ask_message_content_deps(
        self,
    ) -> discord_plain_ask.PlainAskMessageContentDeps[discord_plain_ask.PlainAskPreparedMessage, int]:
        return discord_plain_ask.PlainAskMessageContentDeps(
            build_prompt_with_discord_attachments=self.build_prompt_with_discord_attachments,
            send_chunks=self.send_chunks,
            log=self.log,
        )

    def make_startup_probe_deps(
        self,
        bot: ReadyBot[ChannelT],
    ) -> discord_startup_probe.StartupProbeDeps[ChannelT]:
        return discord_startup_probe.StartupProbeDeps(
            get_cached_channel_or_thread=bot.get_cached_channel_or_thread,
            fetch_channel=bot.fetch_channel,
            delivery_exceptions=self.delivery_exceptions,
            is_messageable=self.is_messageable,
            is_allowed_message_channel=bot.is_allowed_message_channel,
            log=self.log,
        )

    def make_stale_busy_choice_cleanup_deps(
        self,
        bot: ReadyBot[ChannelT],
    ) -> discord_ready_cleanup.StaleBusyChoiceCleanupDeps[ChannelT]:
        return discord_ready_cleanup.StaleBusyChoiceCleanupDeps(
            get_startup_probe_targets=lambda: self.get_startup_probe_targets(
                bot.allowed_channel_ids,
                bot.startup_channel_id,
            ),
            get_cached_channel_or_thread=bot.get_cached_channel_or_thread,
            fetch_channel=bot.fetch_channel,
            delivery_exceptions=self.delivery_exceptions,
            is_messageable=self.is_messageable,
            cleanup_channel=self.cleanup_stale_busy_choice_components_in_channel,
            log=self.log,
        )

    def make_ready_maintenance_deps(self, bot: ReadyBot[ChannelT]) -> discord_ready_cleanup.ReadyMaintenanceDeps:
        stale_cleanup = getattr(bot, "cleanup_stale_busy_choice_components", None)
        return discord_ready_cleanup.ReadyMaintenanceDeps(
            cleanup_expired_busy_choices=self.cleanup_expired_busy_choices,
            cleanup_expired_persistent_component_claims=self.cleanup_expired_persistent_component_claims,
            cleanup_processed_messages=self.cleanup_processed_messages,
            cleanup_session_mirror_events=self.cleanup_session_mirror_events,
            cleanup_stale_busy_choice_components=(
                cast(discord_ready_cleanup.AsyncReadyCleanup, stale_cleanup)
                if callable(stale_cleanup)
                else None
            ),
            log=self.log,
        )

    def make_startup_diagnostics_deps(
        self,
        bot: ReadyBot[ChannelT],
        probe_channel_access: discord_startup_probe.StartupProbeRunner,
    ) -> discord_startup_probe.StartupDiagnosticsDeps:
        return discord_startup_probe.StartupDiagnosticsDeps(
            allowed_channel_ids=bot.allowed_channel_ids,
            startup_channel_id=bot.startup_channel_id,
            get_probe_targets=self.get_startup_probe_targets,
            get_probe_timeout=self.get_startup_probe_timeout,
            probe_channel_access=probe_channel_access,
            delivery_exceptions=self.delivery_exceptions,
            format_traceback=self.format_traceback,
            log=self.log,
        )

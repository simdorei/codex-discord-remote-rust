from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command
import codex_discord_session_mirror_delegation as discord_session_mirror_delegation
import codex_discord_session_mirror_output_targets as discord_session_mirror_output_targets
ModuleValue: TypeAlias = object


class PrefixMirrorBuilder(Protocol):
    def __call__(
        self,
        bot: ModuleValue,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str: ...


class PrimeCursor(Protocol):
    def __call__(self, target_thread_id: str | None) -> int | None: ...


@dataclass(frozen=True, slots=True)
class BotSessionMirrorDelegationRuntime:
    module: ModuleType

    def should_delegate_output_to_session_mirror(
        self,
        channel: discord_session_mirror_delegation.SessionMirrorOutputChannel,
        target_thread_id: str | None,
    ) -> bool:
        return discord_session_mirror_delegation.should_delegate_output_to_session_mirror(
            channel,
            target_thread_id,
            session_mirror_enabled_func=self._session_mirror_enabled,
            get_mirrored_codex_thread_id_func=cast(
                Callable[[int | None], str | None],
                getattr(self.module, "get_mirrored_codex_thread_id"),
            ),
            bridge_module=cast(
                discord_session_mirror_delegation.MirrorStatusBridge,
                getattr(self.module, "BRIDGE_MIRROR_STATUS"),
            ),
            expected_exceptions=(OSError, RuntimeError, sqlite3.Error),
            log_func=cast(Callable[[str], None], getattr(self.module, "log_line")),
        )

    def should_delegate_session_mirror_output(
        self,
        channel: discord_session_mirror_delegation.SessionMirrorOutputChannel,
        target_thread_id: str | None,
    ) -> bool:
        return self.should_delegate_output_to_session_mirror(channel, target_thread_id)

    async def prepare_session_mirror_delegation(
        self,
        channel: discord_session_mirror_delegation.SessionMirrorOutputChannel,
        target_thread_id: str | None,
    ) -> bool:
        return await discord_session_mirror_delegation.prepare_session_mirror_delegation(
            channel,
            target_thread_id,
            should_delegate_func=self._should_delegate_output_to_session_mirror,
            prime_cursor_func=cast(PrimeCursor, getattr(self.module, "prime_session_mirror_cursor_for_target")),
        )

    async def prepare_mapped_session_mirror_output(
        self,
        channel: discord_session_mirror_output_targets.SessionMirrorOutputChannel,
        target_thread_id: str | None,
    ) -> bool:
        return await discord_session_mirror_output_targets.prepare_mapped_session_mirror_output(
            channel,
            target_thread_id,
            deps=discord_session_mirror_output_targets.PrepareMappedSessionMirrorOutputDeps(
                session_mirror_enabled=self._session_mirror_enabled,
                get_mirrored_codex_thread_id=cast(
                    Callable[[int | None], str | None],
                    getattr(self.module, "get_mirrored_codex_thread_id"),
                ),
                prime_session_mirror_cursor_for_target=cast(
                    Callable[[str | None], int | None],
                    getattr(self.module, "prime_session_mirror_cursor_for_target"),
                ),
                session_mirror_rollout_path_missing=cast(
                    Callable[[str | None], bool],
                    getattr(self.module, "session_mirror_rollout_path_missing"),
                ),
                activate_session_mirror_output_target=cast(
                    Callable[[str | None], None],
                    getattr(self.module, "activate_session_mirror_output_target"),
                ),
                activate_pending_session_mirror_output_target=cast(
                    Callable[[str | None], None],
                    getattr(self.module, "activate_pending_session_mirror_output_target"),
                ),
                expected_exceptions=(OSError, RuntimeError, sqlite3.Error),
                log=cast(Callable[[str], None], getattr(self.module, "log_line")),
            ),
        )

    def build_prefix_mirror_list(
        self,
        bot: discord_prefix_mirror_commands.MirrorCommandBot,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        builder = cast(PrefixMirrorBuilder, getattr(self.module, "build_mirror_list_for_prefix"))
        return builder(bot, limit, channel_id=channel_id)

    def build_prefix_mirror_check(
        self,
        bot: discord_prefix_mirror_commands.MirrorCommandBot,
        limit: int | None = None,
        *,
        channel_id: int | None = None,
    ) -> str:
        builder = cast(PrefixMirrorBuilder, getattr(self.module, "build_mirror_check_for_prefix"))
        return builder(bot, limit, channel_id=channel_id)

    async def prepare_prefix_mapped_session_mirror_output(
        self,
        channel: discord_prefix_steer_command.ChannelLike,
        target_thread_id: str | None,
    ) -> bool:
        return await self.prepare_mapped_session_mirror_output(
            cast(discord_session_mirror_output_targets.SessionMirrorOutputChannel, channel),
            target_thread_id,
        )

    async def prepare_prefix_session_mirror_delegation(
        self,
        channel: discord_prefix_steer_command.ChannelLike,
        target_thread_id: str | None,
    ) -> bool:
        return await self.prepare_session_mirror_delegation(
            cast(discord_session_mirror_delegation.SessionMirrorOutputChannel, channel),
            target_thread_id,
        )

    def _session_mirror_enabled(self) -> bool:
        is_enabled = cast(Callable[[], bool], getattr(self.module, "discord_session_mirror_enabled"))
        return is_enabled()

    def _should_delegate_output_to_session_mirror(
        self,
        channel: discord_session_mirror_delegation.SessionMirrorOutputChannel,
        target_thread_id: str | None,
    ) -> bool:
        should_delegate = cast(
            discord_session_mirror_delegation.ShouldDelegateFunc[
                discord_session_mirror_delegation.SessionMirrorOutputChannel
            ],
            getattr(self.module, "should_delegate_output_to_session_mirror"),
        )
        return should_delegate(channel, target_thread_id)

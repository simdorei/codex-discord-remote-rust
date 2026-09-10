from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_cursor as discord_session_mirror_cursor
import codex_discord_session_mirror_output_targets as discord_session_mirror_output_targets
import codex_discord_store as discord_store

GetDbPathFunc = Callable[[], Path]
GetSessionMirrorStateFunc = Callable[[], discord_session_mirror.SessionMirrorState]
SessionMirrorEnabledFunc = Callable[[], bool]
SessionMirrorCursorGetter = Callable[[str, str, int], int]
SessionMirrorCursorUpdater = Callable[[str, str, int], None]
OutputTargetPredicate = Callable[[str], bool]
ExpiredOutputTargetHandler = Callable[[str, float], None]
TimeFunc = Callable[[], float]
FormatExceptionFunc = Callable[[], str]
LogFunc = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class SessionMirrorStateRuntime:
    get_db_path: GetDbPathFunc
    get_session_mirror_state: GetSessionMirrorStateFunc
    session_mirror_enabled: SessionMirrorEnabledFunc
    choose_thread: discord_session_mirror_cursor.SessionMirrorCursorChooser
    get_or_init_cursor: SessionMirrorCursorGetter
    update_cursor: SessionMirrorCursorUpdater
    is_active_output_target: OutputTargetPredicate
    time_now: TimeFunc
    preserve_seconds: float
    active_ttl_seconds: float
    on_expired_output_target: ExpiredOutputTargetHandler
    exception_types: discord_session_mirror_cursor.MirrorCursorExceptionTypes
    format_exception: FormatExceptionFunc
    log: LogFunc

    def get_or_init_session_mirror_cursor(
        self,
        codex_thread_id: str,
        rollout_path: str,
        initial_cursor: int,
    ) -> int:
        return discord_store.get_or_init_session_mirror_cursor(
            self.get_db_path(),
            codex_thread_id,
            rollout_path,
            initial_cursor,
        )

    def update_session_mirror_cursor(self, codex_thread_id: str, rollout_path: str, cursor: int) -> None:
        discord_store.update_session_mirror_cursor(
            self.get_db_path(),
            codex_thread_id,
            rollout_path,
            cursor,
        )

    def prime_session_mirror_cursor_for_target(self, target_thread_id: str | None) -> int | None:
        return discord_session_mirror_cursor.prime_session_mirror_cursor_for_target(
            target_thread_id,
            deps=discord_session_mirror_cursor.PrimeSessionMirrorCursorDeps(
                session_mirror_enabled=self.session_mirror_enabled,
                choose_thread=self.choose_thread,
                get_session_mirror_offset=lambda codex_thread_id: discord_store.get_session_mirror_offset(
                    self.get_db_path(),
                    codex_thread_id,
                ),
                get_or_init_session_mirror_cursor=self.get_or_init_cursor,
                update_session_mirror_cursor=self.update_cursor,
                is_active_output_target=self.is_active_output_target,
                preserve_seconds=self.preserve_seconds,
                time_now=self.time_now,
                exception_types=self.exception_types,
                format_exception=self.format_exception,
                log=self.log,
            ),
        )

    def activate_session_mirror_output_target(self, target_thread_id: str | None) -> None:
        discord_session_mirror_output_targets.activate_session_mirror_output_target(
            self.get_session_mirror_state(),
            target_thread_id,
            active_ttl_seconds=self.active_ttl_seconds,
            on_expired=self.on_expired_output_target,
        )

    def activate_pending_session_mirror_output_target(self, target_thread_id: str | None) -> None:
        discord_session_mirror_output_targets.activate_pending_session_mirror_output_target(
            self.get_session_mirror_state(),
            target_thread_id,
            active_ttl_seconds=self.active_ttl_seconds,
            on_expired=self.on_expired_output_target,
        )

    def deactivate_session_mirror_output_target(self, target_thread_id: str | None) -> None:
        discord_session_mirror_output_targets.deactivate_session_mirror_output_target(
            self.get_session_mirror_state(),
            target_thread_id,
        )

    def get_session_mirror_output_target_activation(
        self,
        target_thread_id: str | None,
    ) -> float | None:
        return discord_session_mirror_output_targets.get_session_mirror_output_target_activation(
            self.get_session_mirror_state(),
            target_thread_id,
        )

    def deactivate_session_mirror_output_target_if_unchanged(
        self,
        target_thread_id: str | None,
        expected_activation: float | None,
    ) -> bool:
        return discord_session_mirror_output_targets.deactivate_session_mirror_output_target_if_unchanged(
            self.get_session_mirror_state(),
            target_thread_id,
            expected_activation,
        )

    def clear_expiring_session_mirror_output_target(
        self,
        target_thread_id: str,
        expected_activation: float,
    ) -> None:
        discord_session_mirror_output_targets.clear_expiring_session_mirror_output_target(
            self.get_session_mirror_state(),
            target_thread_id,
            expected_activation,
        )

    def is_active_session_mirror_output_target(self, target_thread_id: str | None) -> bool:
        return discord_session_mirror_output_targets.is_active_session_mirror_output_target(
            self.get_session_mirror_state(),
            target_thread_id,
            active_ttl_seconds=self.active_ttl_seconds,
            on_expired=self.on_expired_output_target,
        )

    def is_pending_session_mirror_cursor_target(self, target_thread_id: str | None) -> bool:
        return discord_session_mirror_output_targets.is_pending_session_mirror_cursor_target(
            self.get_session_mirror_state(),
            target_thread_id,
            active_ttl_seconds=self.active_ttl_seconds,
            on_expired=self.on_expired_output_target,
        )

    def clear_pending_session_mirror_cursor_target(self, target_thread_id: str | None) -> None:
        discord_session_mirror_output_targets.clear_pending_session_mirror_cursor_target(
            self.get_session_mirror_state(),
            target_thread_id,
        )

    def session_mirror_rollout_path_missing(self, target_thread_id: str | None) -> bool:
        if not target_thread_id:
            return False
        try:
            codex_thread = self.choose_thread(target_thread_id, None)
            return not Path(codex_thread.rollout_path).exists()
        except self.exception_types as exc:
            self.log(
                f"session_mirror_output_prepare_failed target={target_thread_id} "
                + f"reason=thread_unavailable error_type={type(exc).__name__}"
            )
            return False

    def claim_session_mirror_event(self, event_digest: str, codex_thread_id: str) -> bool:
        return discord_store.claim_session_mirror_event(
            self.get_db_path(),
            event_digest,
            codex_thread_id,
        )

    def has_session_mirror_event(self, event_digest: str, codex_thread_id: str) -> bool:
        return discord_store.has_session_mirror_event(
            self.get_db_path(),
            event_digest,
            codex_thread_id,
        )

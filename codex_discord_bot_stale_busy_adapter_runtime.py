from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Protocol, cast

import codex_discord_busy as discord_busy
import codex_discord_stale_busy_steer as discord_stale_busy_steer


class PendingInteractiveState(Protocol):
    pass


class StaleBusyBridge(Protocol):
    def choose_thread(self, thread_id: str, fallback: str | None = None) -> discord_stale_busy_steer.ThreadLike: ...

    def is_thread_busy(self, path: Path) -> bool: ...

    def get_pending_interactive_state_from_session(self, path: Path) -> PendingInteractiveState | None: ...

    def session_file_age_seconds(self, path: Path) -> float | None: ...

    def get_thread_workspace_ref(self, thread: discord_stale_busy_steer.ThreadLike) -> str: ...


class StaleBusyRuntimeConfig(Protocol):
    def get_stale_busy_steer_block_seconds(self, *, default: float) -> float: ...


@dataclass(frozen=True, slots=True)
class BotStaleBusyAdapterRuntime:
    module: ModuleType

    def get_stale_busy_steer_block_info(self, target_thread_id: str | None) -> tuple[str, str, float] | None:
        bridge = cast(StaleBusyBridge, getattr(self.module, "BRIDGE_STALE_BUSY_STEER"))
        try:
            return discord_stale_busy_steer.get_stale_busy_steer_block_info(
                target_thread_id,
                resolve_target_ref=cast(
                    Callable[[str | None], tuple[str | None, str]],
                    getattr(self.module, "resolve_target_ref"),
                ),
                choose_thread=lambda thread_id: bridge.choose_thread(thread_id, None),
                is_thread_busy=bridge.is_thread_busy,
                get_pending_interactive_state=bridge.get_pending_interactive_state_from_session,
                session_file_age_seconds=bridge.session_file_age_seconds,
                get_thread_workspace_ref=bridge.get_thread_workspace_ref,
                stale_seconds=self._stale_seconds(),
            )
        except (OSError, RuntimeError, sqlite3.Error) as exc:
            self._log(f"stale_busy_steer_check_unavailable target={target_thread_id or '-'} error={exc}")
            return None

    async def send_stale_busy_steer_block_message(
        self,
        channel: discord_stale_busy_steer.StaleBusySteerChannel,
        prompt: str,
        target_thread_id: str | None,
        *,
        reason: str,
    ) -> bool:
        return await discord_stale_busy_steer.send_stale_busy_steer_block_message(
            channel,
            prompt,
            target_thread_id,
            reason=reason,
            deps=discord_stale_busy_steer.StaleBusySteerBlockDeps(
                get_block_info=cast(
                    Callable[[str | None], tuple[str, str, float] | None],
                    getattr(self.module, "get_stale_busy_steer_block_info"),
                ),
                build_message=self.build_stale_busy_steer_block_message,
                send_chunks=cast(discord_stale_busy_steer.SendChunksFunc, getattr(self.module, "send_chunks")),
                log=self._log,
                format_log_text_len=cast(Callable[[str], str], getattr(self.module, "format_log_text_len")),
            ),
        )

    def build_stale_busy_steer_block_message(
        self,
        prompt: str,
        *,
        target_ref: str,
        age_seconds: float,
    ) -> str:
        return discord_busy.build_stale_busy_steer_block_message(
            prompt,
            target_ref=target_ref,
            age_seconds=age_seconds,
            fit_single_message_func=cast(Callable[[str], str], getattr(self.module, "fit_single_message")),
        )

    def _stale_seconds(self) -> float:
        runtime_config = cast(StaleBusyRuntimeConfig, getattr(self.module, "discord_runtime_config"))
        default = cast(float, getattr(self.module, "STALE_BUSY_STEER_BLOCK_SECONDS"))
        return runtime_config.get_stale_busy_steer_block_seconds(default=default)

    def _log(self, message: str) -> None:
        log = cast(Callable[[str], None], getattr(self.module, "log_line"))
        log(message)

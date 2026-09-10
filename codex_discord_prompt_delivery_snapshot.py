from __future__ import annotations

import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_prompt_busy_result as discord_prompt_busy_result
from codex_thread_models import ThreadInfo

LogFunc = Callable[[str], None]


class PromptDeliverySnapshotBridge(Protocol):
    def choose_thread(self, thread_id: str, cwd: str | None) -> ThreadInfo: ...

    def snapshot_recent_session_offsets(
        self,
        *,
        limit: int,
        include_threads: list[ThreadInfo],
    ) -> discord_prompt_busy_result.RecentOffsets: ...


@dataclass(frozen=True, slots=True)
class PromptDeliverySnapshotDeps:
    bridge: PromptDeliverySnapshotBridge
    log: LogFunc


def snapshot_ask_prompt_delivery_state(
    target_thread_id: str | None,
    *,
    deps: PromptDeliverySnapshotDeps,
) -> tuple[ThreadInfo | None, discord_prompt_busy_result.RecentOffsets]:
    if not target_thread_id:
        return None, {}
    try:
        target_thread = deps.bridge.choose_thread(target_thread_id, None)
        recent_offsets = deps.bridge.snapshot_recent_session_offsets(
            limit=10,
            include_threads=[target_thread],
        )
        return target_thread, recent_offsets
    except (OSError, RuntimeError, sqlite3.Error) as exc:
        deps.log(
            f"ask_delivery_snapshot_unavailable target={target_thread_id or '-'} "
            + f"error={str(exc)[:200]}"
        )
        return None, {}

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_store as discord_store
from codex_discord_session_mirror_detail import SessionMirrorDetailMode


@dataclass(frozen=True, slots=True)
class SessionMirrorDetailRuntime:
    get_db_path: Callable[[], Path]

    def get_mode(self, codex_thread_id: str) -> SessionMirrorDetailMode:
        return discord_store.get_session_mirror_detail_mode(
            self.get_db_path(),
            codex_thread_id,
        )

    def set_mode(
        self,
        codex_thread_id: str,
        detail_mode: SessionMirrorDetailMode,
    ) -> None:
        discord_store.set_session_mirror_detail_mode(
            self.get_db_path(),
            codex_thread_id,
            detail_mode,
        )

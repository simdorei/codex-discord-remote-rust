from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import cast

from codex_discord_session_mirror_detail import SessionMirrorDetailMode
from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


def get_session_mirror_detail_mode(
    db_path: Path,
    codex_thread_id: str,
) -> SessionMirrorDetailMode:
    with connect_store(db_path) as conn:
        init_store_schema(conn)
        row = cast(
            tuple[str] | None,
            conn.execute(
                "SELECT detail_mode FROM session_mirror_details "
                + "WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
    if row is None:
        return SessionMirrorDetailMode.SEND
    return SessionMirrorDetailMode(row[0])


def set_session_mirror_detail_mode(
    db_path: Path,
    codex_thread_id: str,
    detail_mode: SessionMirrorDetailMode,
) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)
        mapped_row = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT 1 FROM mirror_threads WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
        if mapped_row is None:
            raise KeyError(f"Mirror thread is not mapped: {codex_thread_id}")
        _ = conn.execute(
            "INSERT INTO session_mirror_details "
            + "(codex_thread_id, detail_mode) VALUES (?, ?) "
            + "ON CONFLICT(codex_thread_id) DO UPDATE SET "
            + "detail_mode = excluded.detail_mode",
            (str(codex_thread_id), detail_mode.value),
        )

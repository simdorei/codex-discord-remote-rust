from __future__ import annotations

import sqlite3
import time
from pathlib import Path
from typing import cast

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def get_or_init_session_mirror_cursor(
    db_path: Path,
    codex_thread_id: str,
    rollout_path: str,
    initial_cursor: int,
    *,
    now: float | None = None,
) -> int:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[str | None, int | None] | None,
            conn.execute(
                "SELECT rollout_path, cursor "
                + "FROM codex_session_mirror_offsets "
                + "WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
        if row and str(row[0] or "") == str(rollout_path):
            return int(row[1] or 0)
        _ = conn.execute(
            "INSERT OR REPLACE INTO codex_session_mirror_offsets ("
            + "codex_thread_id, rollout_path, cursor, updated_at"
            + ") VALUES (?, ?, ?, ?)",
            (str(codex_thread_id), str(rollout_path), int(initial_cursor), current),
        )
    return int(initial_cursor)


def get_session_mirror_offset(
    db_path: Path,
    codex_thread_id: str,
) -> tuple[str, int, float] | None:
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[str | None, int | None, float | None] | None,
            conn.execute(
                "SELECT rollout_path, cursor, updated_at "
                + "FROM codex_session_mirror_offsets "
                + "WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
    if not row:
        return None
    return str(row[0] or ""), int(row[1] or 0), float(row[2] or 0.0)


def update_session_mirror_cursor(
    db_path: Path,
    codex_thread_id: str,
    rollout_path: str,
    cursor: int,
    *,
    now: float | None = None,
) -> None:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        _ = conn.execute(
            "INSERT OR REPLACE INTO codex_session_mirror_offsets ("
            + "codex_thread_id, rollout_path, cursor, updated_at"
            + ") VALUES (?, ?, ?, ?)",
            (str(codex_thread_id), str(rollout_path), int(cursor), current),
        )

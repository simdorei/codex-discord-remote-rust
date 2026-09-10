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


def claim_session_mirror_event(
    db_path: Path,
    event_digest: str,
    codex_thread_id: str,
    *,
    now: float | None = None,
) -> bool:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        result = conn.execute(
            "INSERT OR IGNORE INTO codex_session_mirror_events ("
            + "event_digest, codex_thread_id, created_at"
            + ") VALUES (?, ?, ?)",
            (str(event_digest), str(codex_thread_id), current),
        )
        return result.rowcount == 1


def has_session_mirror_event(
    db_path: Path,
    event_digest: str,
    codex_thread_id: str,
) -> bool:
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT 1 FROM codex_session_mirror_events "
                + "WHERE event_digest = ? AND codex_thread_id = ?",
                (str(event_digest), str(codex_thread_id)),
            ).fetchone(),
        )
    return row is not None


def cleanup_session_mirror_events(
    db_path: Path,
    *,
    retention_seconds: float,
    now: float | None = None,
) -> int:
    current = time.time() if now is None else now
    cutoff = current - retention_seconds
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        result = conn.execute(
            "DELETE FROM codex_session_mirror_events WHERE created_at < ?",
            (cutoff,),
        )
        return result.rowcount

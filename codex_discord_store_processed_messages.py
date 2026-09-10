from __future__ import annotations

import sqlite3
import time
from pathlib import Path
from typing import cast

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


def _init_store_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def cleanup_processed_discord_messages(db_path: Path, *, retention_seconds: float, now: float | None = None) -> int:
    _ = (retention_seconds, now)
    _init_store_db(db_path)
    return 0


def claim_persistent_discord_message_id(db_path: Path, message_id: int, now: float | None = None) -> bool:
    current = time.time() if now is None else now
    _init_store_db(db_path)
    with connect_store(db_path) as conn:
        result = conn.execute(
            "INSERT OR IGNORE INTO discord_processed_messages (message_id, seen_at) "
            + "VALUES (?, ?)",
            (int(message_id), current),
        )
        return result.rowcount == 1


def is_processed_discord_message_id(db_path: Path, message_id: int) -> bool:
    _init_store_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT 1 FROM discord_processed_messages WHERE message_id = ?",
                (int(message_id),),
            ).fetchone(),
        )
        return row is not None


def mark_processed_discord_message_id(db_path: Path, message_id: int, now: float | None = None) -> None:
    current = time.time() if now is None else now
    _init_store_db(db_path)
    with connect_store(db_path) as conn:
        _ = conn.execute(
            "INSERT OR REPLACE INTO discord_processed_messages (message_id, seen_at) "
            + "VALUES (?, ?)",
            (int(message_id), current),
        )

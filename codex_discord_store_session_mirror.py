from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import cast

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema
from codex_discord_store_session_mirror_events import (
    claim_session_mirror_event as claim_session_mirror_event,
    cleanup_session_mirror_events as cleanup_session_mirror_events,
    has_session_mirror_event as has_session_mirror_event,
)
from codex_discord_store_session_mirror_offsets import (
    get_or_init_session_mirror_cursor as get_or_init_session_mirror_cursor,
    get_session_mirror_offset as get_session_mirror_offset,
    update_session_mirror_cursor as update_session_mirror_cursor,
)


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def get_session_mirror_targets(db_path: Path, *, limit: int = 100) -> list[dict[str, str | int]]:
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        rows = cast(
            list[tuple[str | None, str | None, int, int]],
            conn.execute(
                "SELECT codex_thread_id, thread_title, discord_channel_id, "
                + "discord_thread_id FROM mirror_threads "
                + "ORDER BY updated_at DESC LIMIT ?",
                (int(limit),),
            ).fetchall(),
        )
    return [
        {
            "codex_thread_id": str(row[0] or ""),
            "thread_title": str(row[1] or ""),
            "discord_channel_id": int(row[2]),
            "discord_thread_id": int(row[3]),
        }
        for row in rows
        if row[0] and row[3]
    ]

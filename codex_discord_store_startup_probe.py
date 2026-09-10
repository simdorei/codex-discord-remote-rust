from __future__ import annotations

import sqlite3
from pathlib import Path

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def get_startup_probe_targets(
    db_path: Path,
    allowed_channel_ids: set[int],
    startup_channel_id: int | None,
    *,
    limit: int = 30,
) -> list[tuple[str, int]]:
    seen: set[int] = set()
    targets: list[tuple[str, int]] = []

    def add(label: str, channel_id: int | None) -> None:
        if not channel_id:
            return
        normalized = int(channel_id)
        if normalized in seen or len(targets) >= limit:
            return
        seen.add(normalized)
        targets.append((label, normalized))

    add("startup", startup_channel_id)
    for channel_id in sorted(allowed_channel_ids):
        add("allowed", channel_id)

    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        project_rows = list[tuple[int]](
            conn.execute(
                "SELECT discord_channel_id FROM mirror_projects "
                + "ORDER BY updated_at DESC "
                + "LIMIT ?",
                (limit,),
            ).fetchall(),
        )
        for row in project_rows:
            add("mirror_project", int(row[0]))
        thread_rows = list[tuple[int]](
            conn.execute(
                "SELECT discord_thread_id FROM mirror_threads "
                + "ORDER BY updated_at DESC "
                + "LIMIT ?",
                (limit,),
            ).fetchall(),
        )
        for row in thread_rows:
            add("mirror_thread", int(row[0]))
    return targets

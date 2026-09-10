from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import cast

from codex_discord_store_connection import connect_store
from codex_discord_store_mirror_project_aliases import ProjectKeysMatchFunc
from codex_discord_store_schema import init_store_schema


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def find_mirror_project_row_by_key(
    db_path: Path,
    canonical_project_key: str | None,
    *,
    project_keys_match_func: ProjectKeysMatchFunc,
) -> tuple[int, str] | None:
    canonical = str(canonical_project_key or "").strip()
    if not canonical:
        return None
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int, str | None] | None,
            conn.execute(
                "SELECT discord_channel_id, project_key "
                + "FROM mirror_projects "
                + "WHERE project_key = ?",
                (canonical,),
            ).fetchone(),
        )
        if row:
            return int(row[0]), str(row[1] or "")
        rows = cast(
            list[tuple[int, str | None]],
            conn.execute(
                "SELECT discord_channel_id, project_key "
                + "FROM mirror_projects "
                + "ORDER BY updated_at DESC"
            ).fetchall(),
        )
    for row in rows:
        row_project_key = str(row[1] or "")
        if project_keys_match_func(row_project_key, canonical):
            return int(row[0]), row_project_key
    return None

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from pathlib import Path
from typing import TypeAlias, cast

from codex_discord_store_connection import connect_store

SqlRow: TypeAlias = tuple[object, ...]
SqlParam: TypeAlias = str | int | float | bytes | None
InitMirrorDb: TypeAlias = Callable[[], None]
ProjectKeysMatch: TypeAlias = Callable[[str | None, str | None], bool]


def _fetch_rows(conn: sqlite3.Connection, query: str, params: tuple[SqlParam, ...]) -> list[SqlRow]:
    return cast(list[SqlRow], conn.execute(query, params).fetchall())


def _row_text(row: SqlRow, index: int) -> str:
    return str(row[index] or "")


def _row_int(row: SqlRow, index: int) -> int:
    return int(_row_text(row, index))


def resolve_discord_new_thread_project_channel_id(
    discord_channel_id: int | None,
    project_key: str | None,
    *,
    db_path: Path,
    init_mirror_db_func: InitMirrorDb,
    project_keys_match_func: ProjectKeysMatch,
) -> int | None:
    if not discord_channel_id or not project_key:
        return None
    init_mirror_db_func()
    with connect_store(db_path) as conn:
        thread_rows = _fetch_rows(
            conn,
            "SELECT discord_channel_id, project_key FROM mirror_threads WHERE discord_thread_id = ? ORDER BY updated_at DESC",
            (int(discord_channel_id),),
        )
        for row in thread_rows:
            if project_keys_match_func(_row_text(row, 1), project_key):
                return _row_int(row, 0)
        project_rows = _fetch_rows(
            conn,
            "SELECT discord_channel_id, project_key FROM mirror_projects WHERE discord_channel_id = ? ORDER BY updated_at DESC",
            (int(discord_channel_id),),
        )
    for row in project_rows:
        if project_keys_match_func(_row_text(row, 1), project_key):
            return _row_int(row, 0)
    return None

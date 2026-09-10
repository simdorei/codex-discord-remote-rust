from __future__ import annotations

import sqlite3
import time
from pathlib import Path

from codex_discord_store_connection import connect_store
from codex_discord_store_mirror_project_aliases import (
    ProjectKeysMatchFunc,
    merge_mirror_project_key_aliases,
)
from codex_discord_store_schema import init_store_schema


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def upsert_mirror_project(
    db_path: Path,
    canonical_project_key: str,
    project_name: str,
    channel_id: int,
    *,
    project_keys_match_func: ProjectKeysMatchFunc,
    now: float | None = None,
) -> list[str]:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        merged_aliases = merge_mirror_project_key_aliases(
            conn,
            canonical_project_key,
            project_keys_match_func=project_keys_match_func,
        )
        _ = conn.execute(
            "INSERT OR REPLACE INTO mirror_projects "
            + "(project_key, project_name, discord_channel_id, updated_at) "
            + "VALUES (?, ?, ?, ?)",
            (canonical_project_key, project_name, int(channel_id), current),
        )
    return merged_aliases

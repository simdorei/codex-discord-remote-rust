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


def get_mirrored_codex_thread_id(db_path: Path, discord_channel_id: int | None) -> str | None:
    if not discord_channel_id:
        return None
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[str] | None,
            conn.execute(
                "SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id = ?",
                (int(discord_channel_id),),
            ).fetchone(),
        )
        if row:
            return str(row[0])
        rows = cast(
            list[tuple[str]],
            conn.execute(
                "SELECT codex_thread_id "
                + "FROM mirror_threads "
                + "WHERE discord_channel_id = ? "
                + "ORDER BY updated_at DESC "
                + "LIMIT 2",
                (int(discord_channel_id),),
            ).fetchall(),
        )
    if len(rows) == 1:
        return str(rows[0][0])
    return None


def upsert_mirror_thread(
    db_path: Path,
    codex_thread_id: str,
    canonical_project_key: str,
    thread_name: str,
    project_channel_id: int,
    discord_thread_id: int,
    *,
    now: float | None = None,
) -> None:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        _ = conn.execute(
            "INSERT INTO mirror_threads "
            + "(codex_thread_id, project_key, thread_title, discord_channel_id, "
            + "discord_thread_id, updated_at) "
            + "VALUES (?, ?, ?, ?, ?, ?) "
            + "ON CONFLICT(codex_thread_id) DO UPDATE SET "
            + "project_key = excluded.project_key, "
            + "thread_title = excluded.thread_title, "
            + "discord_channel_id = excluded.discord_channel_id, "
            + "discord_thread_id = excluded.discord_thread_id, "
            + "updated_at = excluded.updated_at",
            (
                str(codex_thread_id),
                canonical_project_key,
                thread_name,
                int(project_channel_id),
                int(discord_thread_id),
                current,
            ),
        )


def get_mirror_thread_row_by_codex_thread_id(
    db_path: Path,
    codex_thread_id: str,
) -> tuple[int, int] | None:
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int, int] | None,
            conn.execute(
                "SELECT discord_channel_id, discord_thread_id "
                + "FROM mirror_threads "
                + "WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
    if not row:
        return None
    return int(row[0]), int(row[1])


def update_mirror_thread_discord_thread_id(
    db_path: Path,
    codex_thread_id: str,
    discord_thread_id: int,
    *,
    now: float | None = None,
) -> tuple[int, int] | None:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int, int] | None,
            conn.execute(
                "SELECT discord_channel_id, discord_thread_id "
                + "FROM mirror_threads "
                + "WHERE codex_thread_id = ?",
                (str(codex_thread_id),),
            ).fetchone(),
        )
        if row is None:
            return None
        _ = conn.execute(
            "UPDATE mirror_threads "
            + "SET discord_thread_id = ?, updated_at = ? "
            + "WHERE codex_thread_id = ?",
            (int(discord_thread_id), current, str(codex_thread_id)),
        )
    return int(row[0]), int(row[1])

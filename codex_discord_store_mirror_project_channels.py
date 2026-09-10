from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import cast

from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def describe_mirrored_project_channel(db_path: Path, discord_channel_id: int | None) -> str:
    if not discord_channel_id:
        return ""
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        project = cast(
            tuple[str] | None,
            conn.execute(
                "SELECT project_name FROM mirror_projects WHERE discord_channel_id = ?",
                (int(discord_channel_id),),
            ).fetchone(),
        )
        if not project:
            return ""
        rows = cast(
            list[tuple[str | None]],
            conn.execute(
                "SELECT thread_title "
                + "FROM mirror_threads "
                + "WHERE discord_channel_id = ? "
                + "ORDER BY updated_at DESC "
                + "LIMIT 10",
                (int(discord_channel_id),),
            ).fetchall(),
        )
    titles = [str(row[0] or "").strip() for row in rows if str(row[0] or "").strip()]
    if len(titles) <= 1:
        return ""
    lines = [
        f"`{project[0]}` project channel has multiple Codex threads.",
        "Send the message inside one of its Discord threads:",
    ]
    lines.extend(f"- {title}" for title in titles)
    return "\n".join(lines)


def get_mirror_project_for_channel(db_path: Path, discord_channel_id: int | None) -> tuple[str, str] | None:
    if not discord_channel_id:
        return None
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        row = cast(
            tuple[str | None, str | None] | None,
            conn.execute(
                "SELECT project_key, project_name FROM mirror_projects WHERE discord_channel_id = ?",
                (int(discord_channel_id),),
            ).fetchone(),
        )
    if not row:
        return None
    return str(row[0] or ""), str(row[1] or "")

from __future__ import annotations

import sqlite3

from codex_discord_mirror_rows import SqliteMirrorRow


def _ordered_scoped_thread_ids(scoped_thread_ids: list[str]) -> list[str]:
    return list(dict.fromkeys(str(thread_id) for thread_id in scoped_thread_ids if str(thread_id)))


def _load_recent_mirror_list_rows(conn: sqlite3.Connection, limit: int) -> list[SqliteMirrorRow]:
    return conn.execute(
        """
        SELECT
            mt.thread_title AS thread_title,
            mt.codex_thread_id AS codex_thread_id,
            mp.project_name AS project_name,
            mt.discord_channel_id AS parent_channel_id,
            mt.discord_thread_id AS discord_thread_id,
            mt.updated_at AS last_seen
        FROM mirror_threads mt
        LEFT JOIN mirror_projects mp ON mp.project_key = mt.project_key
        ORDER BY mt.updated_at DESC
        LIMIT ?
        """,
        (limit,),
    ).fetchall()


def _load_scoped_mirror_list_rows(
    conn: sqlite3.Connection,
    ordered_ids: list[str],
) -> list[SqliteMirrorRow]:
    if not ordered_ids:
        return []
    rows: list[SqliteMirrorRow] = conn.execute(
        """
        SELECT
            mt.thread_title AS thread_title,
            mt.codex_thread_id AS codex_thread_id,
            mp.project_name AS project_name,
            mt.discord_channel_id AS parent_channel_id,
            mt.discord_thread_id AS discord_thread_id,
            mt.updated_at AS last_seen
        FROM mirror_threads mt
        LEFT JOIN mirror_projects mp ON mp.project_key = mt.project_key
        WHERE mt.codex_thread_id IN ({})
        ORDER BY mt.updated_at DESC
        """.format(",".join("?" for _ in ordered_ids)),
        tuple(ordered_ids),
    ).fetchall()
    scoped_order = {thread_id: index for index, thread_id in enumerate(ordered_ids)}
    rows.sort(key=lambda row: scoped_order.get(str(row["codex_thread_id"] or ""), len(scoped_order)))
    return rows


def load_mirror_list_rows(
    conn: sqlite3.Connection,
    limit: int,
    scoped_thread_ids: list[str] | None,
) -> list[SqliteMirrorRow]:
    if scoped_thread_ids is None:
        return _load_recent_mirror_list_rows(conn, limit)
    return _load_scoped_mirror_list_rows(conn, _ordered_scoped_thread_ids(scoped_thread_ids))


def _ordered_scoped_project_keys(scoped_project_keys: set[str]) -> list[str]:
    return list(dict.fromkeys(key for key in scoped_project_keys if key))


def _load_all_mirror_check_rows(conn: sqlite3.Connection) -> list[SqliteMirrorRow]:
    return conn.execute(
        """
        SELECT
            codex_thread_id AS codex_thread_id,
            project_key AS project_key,
            discord_channel_id AS parent_channel_id,
            discord_thread_id AS discord_thread_id,
            updated_at AS last_seen
        FROM mirror_threads
        ORDER BY updated_at DESC
        """
    ).fetchall()


def _load_scoped_mirror_check_rows(
    conn: sqlite3.Connection,
    ordered_keys: list[str],
) -> list[SqliteMirrorRow]:
    if not ordered_keys:
        return []
    return conn.execute(
        """
        SELECT
            codex_thread_id AS codex_thread_id,
            project_key AS project_key,
            discord_channel_id AS parent_channel_id,
            discord_thread_id AS discord_thread_id,
            updated_at AS last_seen
        FROM mirror_threads
        WHERE project_key IN ({})
        ORDER BY updated_at DESC
        """.format(",".join("?" for _ in ordered_keys)),
        tuple(ordered_keys),
    ).fetchall()


def load_mirror_check_rows(
    conn: sqlite3.Connection,
    scoped_project_keys: set[str] | None,
) -> list[SqliteMirrorRow]:
    if scoped_project_keys is None:
        return _load_all_mirror_check_rows(conn)
    ordered_keys = _ordered_scoped_project_keys(scoped_project_keys)
    return _load_scoped_mirror_check_rows(conn, ordered_keys)

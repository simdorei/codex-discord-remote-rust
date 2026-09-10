from __future__ import annotations

import sqlite3
from pathlib import Path
from typing import TypeAlias

import codex_desktop_bridge_state as bridge_state
from codex_thread_models import ThreadInfo


ActiveThreadRow: TypeAlias = tuple[
    str,
    str | None,
    str | None,
    int | None,
    str | None,
    str | None,
    str | None,
    int | None,
]
ArchivedThreadRow: TypeAlias = tuple[
    str,
    str | None,
    str | None,
    int | None,
    str | None,
    str | None,
    str | None,
    int | None,
    int | None,
]


def _connect_readonly(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def _require_state_db() -> Path:
    if not bridge_state.STATE_DB_PATH.exists():
        raise FileNotFoundError(
            f"Codex state database not found: {bridge_state.STATE_DB_PATH}. "
            + "Set CODEX_HOME or CODEX_STATE_DB if your Codex data lives elsewhere."
        )
    return bridge_state.STATE_DB_PATH


def _thread_from_active_row(row: ActiveThreadRow) -> ThreadInfo:
    return ThreadInfo(
        id=row[0],
        title=row[1] or "",
        cwd=row[2] or "",
        updated_at=row[3] or 0,
        rollout_path=row[4] or "",
        model=row[5] or "",
        reasoning_effort=row[6] or "",
        tokens_used=row[7] or 0,
    )


def _thread_from_archived_row(row: ArchivedThreadRow) -> ThreadInfo:
    return ThreadInfo(
        id=row[0],
        title=row[1] or "",
        cwd=row[2] or "",
        updated_at=row[3] or 0,
        rollout_path=row[4] or "",
        model=row[5] or "",
        reasoning_effort=row[6] or "",
        tokens_used=row[7] or 0,
        archived_at=row[8] or 0,
    )


def load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
    query = "\n".join(
        (
            "SELECT id, title, cwd, updated_at, rollout_path, model, reasoning_effort, tokens_used",
            "FROM threads",
            "WHERE archived = 0",
            "ORDER BY updated_at DESC",
        )
    )
    params: tuple[int, ...] = ()
    if limit > 0:
        query += "\n        LIMIT ?"
        params = (limit,)
    with _connect_readonly(_require_state_db()) as conn:
        rows: list[ActiveThreadRow] = conn.execute(query, params).fetchall()

    return [_thread_from_active_row(row) for row in rows]


def load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
    # This scope intentionally represents state DB rows, not session-file guesses.
    query = "\n".join(
        (
            "SELECT id, title, cwd, updated_at, rollout_path, model, reasoning_effort, tokens_used",
            "FROM threads",
            "WHERE archived = 0",
            "  AND source = 'vscode'",
            "  AND COALESCE(thread_source, '') IN ('', 'user')",
            "  AND title != ''",
            "ORDER BY updated_at DESC",
        )
    )
    with _connect_readonly(_require_state_db()) as conn:
        rows: list[ActiveThreadRow] = conn.execute(query).fetchall()

    threads = [_thread_from_active_row(row) for row in rows]
    if limit > 0:
        return threads[:limit]
    return threads


def load_archived_threads(limit: int = 20) -> list[ThreadInfo]:
    query = "\n".join(
        (
            "SELECT id, title, cwd, updated_at, rollout_path, model, reasoning_effort, tokens_used, archived_at",
            "FROM threads",
            "WHERE archived = 1",
            "ORDER BY archived_at DESC, updated_at DESC",
        )
    )
    params: tuple[int, ...] = ()
    if limit > 0:
        query += "\n        LIMIT ?"
        params = (limit,)
    with _connect_readonly(_require_state_db()) as conn:
        rows: list[ArchivedThreadRow] = conn.execute(query, params).fetchall()

    return [_thread_from_archived_row(row) for row in rows]

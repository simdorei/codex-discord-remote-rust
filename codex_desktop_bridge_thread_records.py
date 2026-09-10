from __future__ import annotations

import sqlite3
import time
from pathlib import Path
from typing import TypeAlias

import codex_desktop_bridge_state as bridge_state
from codex_thread_models import ThreadInfo


ThreadRecordRow: TypeAlias = tuple[
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


def load_thread_record_by_id(thread_id: str) -> tuple[ThreadInfo, bool] | None:
    if not bridge_state.STATE_DB_PATH.exists():
        return None
    query = "\n".join(
        (
            "SELECT id, title, cwd, updated_at, rollout_path, model, reasoning_effort, tokens_used, archived",
            "FROM threads",
            "WHERE id = ?",
            "LIMIT 1",
        )
    )
    with _connect_readonly(bridge_state.STATE_DB_PATH) as conn:
        rows: list[ThreadRecordRow] = conn.execute(query, (thread_id,)).fetchall()
    if not rows:
        return None
    row = rows[0]

    thread = ThreadInfo(
        id=row[0],
        title=row[1] or "",
        cwd=row[2] or "",
        updated_at=row[3] or 0,
        rollout_path=row[4] or "",
        model=row[5] or "",
        reasoning_effort=row[6] or "",
        tokens_used=row[7] or 0,
    )
    return thread, bool(row[8])


def wait_for_thread_record(
    thread_id: str,
    *,
    archived: bool | None = None,
    timeout_sec: float = 8.0,
) -> tuple[ThreadInfo, bool] | None:
    deadline = time.time() + max(timeout_sec, 0.0)
    while time.time() < deadline:
        record = load_thread_record_by_id(thread_id)
        if record is not None:
            thread, is_archived = record
            if archived is None or is_archived == archived:
                return thread, is_archived
        time.sleep(0.2)
    return None

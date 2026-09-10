from __future__ import annotations

import sqlite3
from contextlib import closing
from pathlib import Path
from typing import TypeAlias, cast

SQLiteCell: TypeAlias = str | int | float | bytes | None
SQLiteRow: TypeAlias = tuple[SQLiteCell, ...]


def connect_readonly(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def connect_writable(path: Path) -> sqlite3.Connection:
    return sqlite3.connect(path)


def _first_cell_as_text(row: SQLiteRow | None) -> str | None:
    if not row:
        return None
    return str(row[0])


def _fetchone(cursor: sqlite3.Cursor) -> SQLiteRow | None:
    return cast(SQLiteRow | None, cursor.fetchone())


def count_active_threads(path: Path) -> int:
    with closing(connect_readonly(path)) as conn:
        value_text = _first_cell_as_text(_fetchone(conn.execute("SELECT COUNT(*) FROM threads WHERE archived = 0")))
    if value_text is None:
        return 0
    return int(value_text or "0")

from __future__ import annotations

import sqlite3
from collections.abc import Callable
from typing import cast

ProjectKeysMatchFunc = Callable[[str | None, str | None], bool]


def merge_mirror_project_key_aliases(
    conn: sqlite3.Connection,
    canonical_project_key: str,
    *,
    project_keys_match_func: ProjectKeysMatchFunc,
) -> list[str]:
    if not canonical_project_key:
        return []
    rows = cast(
        list[tuple[str | None]],
        conn.execute("SELECT project_key FROM mirror_projects").fetchall(),
    )
    aliases = [
        str(row[0] or "")
        for row in rows
        if str(row[0] or "")
        and str(row[0] or "") != canonical_project_key
        and project_keys_match_func(str(row[0] or ""), canonical_project_key)
    ]
    for alias in aliases:
        _ = conn.execute(
            "UPDATE mirror_threads SET project_key = ? WHERE project_key = ?",
            (canonical_project_key, alias),
        )
        _ = conn.execute("DELETE FROM mirror_projects WHERE project_key = ?", (alias,))
    return aliases

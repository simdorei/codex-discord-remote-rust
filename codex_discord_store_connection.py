from __future__ import annotations

from pathlib import Path
import sqlite3
import time
import uuid


STORE_BUSY_TIMEOUT_MILLISECONDS = 5_000
STORE_BACKUP_DIRECTORY_NAME = ".codex-discord-backups"


def connect_store(db_path: str | Path) -> sqlite3.Connection:
    conn = sqlite3.connect(db_path, timeout=STORE_BUSY_TIMEOUT_MILLISECONDS / 1_000)
    _ = conn.execute(f"PRAGMA busy_timeout = {STORE_BUSY_TIMEOUT_MILLISECONDS}")
    return conn


def backup_store_before_migration(
    conn: sqlite3.Connection,
    *,
    from_version: int,
    to_version: int,
) -> Path | None:
    source_path = _main_database_path(conn)
    if source_path is None or not source_path.exists() or source_path.stat().st_size == 0:
        return None

    backup_dir = source_path.parent / STORE_BACKUP_DIRECTORY_NAME
    backup_dir.mkdir(parents=True, exist_ok=True)
    timestamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    unique = uuid.uuid4().hex[:12]
    backup_path = backup_dir / (
        f"{source_path.stem}.v{from_version}-to-v{to_version}.{timestamp}.{unique}.sqlite"
    )
    try:
        with sqlite3.connect(backup_path) as backup_conn:
            conn.backup(backup_conn)
    except BaseException:
        backup_path.unlink(missing_ok=True)
        raise
    return backup_path


def _main_database_path(conn: sqlite3.Connection) -> Path | None:
    for _sequence, name, raw_path in conn.execute("PRAGMA database_list").fetchall():
        if str(name) == "main" and str(raw_path):
            return Path(str(raw_path)).resolve()
    return None

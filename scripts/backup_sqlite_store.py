from __future__ import annotations

import argparse
import sqlite3
import sys
import uuid
from datetime import UTC, datetime
from pathlib import Path


BACKUP_DIRECTORY = ".codex-discord-backups"


def _integrity_check(connection: sqlite3.Connection) -> None:
    rows = [str(row[0]) for row in connection.execute("PRAGMA integrity_check")]
    if rows != ["ok"]:
        raise RuntimeError(f"SQLite integrity check failed: {rows!r}")


def _schema_version(connection: sqlite3.Connection) -> int:
    row = connection.execute("PRAGMA user_version").fetchone()
    if row is None:
        raise RuntimeError("SQLite user_version was unavailable")
    return int(row[0])


def backup_store(source: Path) -> Path:
    source = source.resolve()
    if not source.is_file():
        raise FileNotFoundError(f"store database was not found: {source}")

    backup_directory = source.parent / BACKUP_DIRECTORY
    backup_directory.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(source, timeout=30.0) as source_connection:
        source_connection.execute("PRAGMA busy_timeout = 30000")
        _integrity_check(source_connection)
        expected_version = _schema_version(source_connection)
        stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        unique = uuid.uuid4().hex[:12]
        target = backup_directory / (
            f"{source.stem}.v{expected_version}-manual-python-rollback."
            f"{stamp}.{unique}.sqlite"
        )
        try:
            with sqlite3.connect(target, timeout=30.0) as target_connection:
                source_connection.backup(target_connection)
                target_connection.commit()
            with sqlite3.connect(target, timeout=30.0) as verification:
                _integrity_check(verification)
                actual_version = _schema_version(verification)
                if actual_version != expected_version:
                    raise RuntimeError(
                        "SQLite backup schema version mismatch: "
                        f"expected={expected_version} actual={actual_version}"
                    )
        except BaseException:
            target.unlink(missing_ok=True)
            raise
    return target


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Create and verify an online SQLite store backup."
    )
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    try:
        target = backup_store(args.source)
    except Exception as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print(f"backup_created path={target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

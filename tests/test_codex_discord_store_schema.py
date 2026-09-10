from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast
from unittest.mock import patch

import codex_discord_store as store
import codex_discord_store_schema as store_schema


class StoreSchemaTests(unittest.TestCase):
    def test_store_schema_helper_creates_expected_tables(self) -> None:
        with sqlite3.connect(":memory:") as conn:
            store_schema.init_store_schema(conn)
            table_rows = cast(
                list[tuple[str]],
                conn.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                ).fetchall(),
            )
            tables = {
                row[0] for row in table_rows
            }

        self.assertEqual(tables, set(store_schema.STORE_SCHEMA_TABLES))

    def test_schema_records_latest_version_and_passes_integrity_check(self) -> None:
        with sqlite3.connect(":memory:") as conn:
            store_schema.init_store_schema(conn)

            version = int(conn.execute("PRAGMA user_version").fetchone()[0])
            integrity = str(conn.execute("PRAGMA integrity_check").fetchone()[0])

        self.assertEqual(version, store_schema.LATEST_STORE_SCHEMA_VERSION)
        self.assertEqual(integrity, "ok")

    def test_public_init_mirror_db_preserves_representative_columns(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            store.init_mirror_db(db_path)
            with sqlite3.connect(db_path) as conn:
                project_rows = cast(
                    list[tuple[int, str, str, int, object, int]],
                    conn.execute("PRAGMA table_info(mirror_projects)").fetchall(),
                )
                busy_choice_rows = cast(
                    list[tuple[int, str, str, int, object, int]],
                    conn.execute("PRAGMA table_info(busy_choices)").fetchall(),
                )
                session_event_rows = cast(
                    list[tuple[int, str, str, int, object, int]],
                    conn.execute(
                        "PRAGMA table_info(codex_session_mirror_events)"
                    ).fetchall(),
                )
                project_columns = {
                    row[1] for row in project_rows
                }
                busy_choice_columns = {
                    row[1] for row in busy_choice_rows
                }
                session_event_columns = {
                    row[1] for row in session_event_rows
                }
                detail_columns = {
                    row[1]
                    for row in conn.execute(
                        "PRAGMA table_info(session_mirror_details)"
                    ).fetchall()
                }

        self.assertEqual(
            project_columns,
            {"project_key", "project_name", "discord_channel_id", "updated_at"},
        )
        self.assertEqual(
            busy_choice_columns,
            {
                "choice_id",
                "owner_user_id",
                "channel_id",
                "target_thread_id",
                "prompt",
                "allow_steer",
                "created_at",
                "expires_at",
                "claimed_at",
            },
        )
        self.assertEqual(
            session_event_columns,
            {"event_digest", "codex_thread_id", "created_at"},
        )
        self.assertEqual(
            detail_columns,
            {
                "codex_thread_id",
                "detail_mode",
            },
        )

    def test_legacy_database_is_backed_up_once_before_migration(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute("CREATE TABLE legacy_value (value TEXT NOT NULL)")
                _ = conn.execute("INSERT INTO legacy_value VALUES ('preserved')")

            store.init_mirror_db(db_path)
            store.init_mirror_db(db_path)
            backups = list((db_path.parent / ".codex-discord-backups").glob("*.sqlite"))
            with sqlite3.connect(backups[0]) as backup_conn:
                backup_value = str(
                    backup_conn.execute("SELECT value FROM legacy_value").fetchone()[0]
                )
                backup_version = int(backup_conn.execute("PRAGMA user_version").fetchone()[0])

        self.assertEqual(len(backups), 1)
        self.assertEqual(backup_value, "preserved")
        self.assertEqual(backup_version, 0)

    def test_failed_migration_rolls_back_and_leaves_backup(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute("CREATE TABLE legacy_value (value TEXT NOT NULL)")

            with patch.object(
                store_schema,
                "assert_store_integrity",
                side_effect=store_schema.StoreIntegrityError("forced failure"),
            ):
                with self.assertRaises(store_schema.StoreIntegrityError):
                    store.init_mirror_db(db_path)

            with sqlite3.connect(db_path) as conn:
                version = int(conn.execute("PRAGMA user_version").fetchone()[0])
                tables = {
                    str(row[0])
                    for row in conn.execute(
                        "SELECT name FROM sqlite_master WHERE type = 'table'"
                    ).fetchall()
                }
            backups = list((db_path.parent / ".codex-discord-backups").glob("*.sqlite"))

        self.assertEqual(version, 0)
        self.assertEqual(tables, {"legacy_value"})
        self.assertEqual(len(backups), 1)


if __name__ == "__main__":
    _ = unittest.main()

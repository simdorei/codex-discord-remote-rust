from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex_discord_store as store
from codex_discord_session_mirror_detail import SessionMirrorDetailMode


class SessionMirrorDetailStoreTests(unittest.TestCase):
    def test_new_mirror_thread_defaults_to_send_and_keeps_selected_mode(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            store.upsert_mirror_thread(
                db_path,
                "thread-1",
                "project-1",
                "Thread 1",
                100,
                101,
                now=1.0,
            )

            self.assertIs(
                store.get_session_mirror_detail_mode(db_path, "thread-1"),
                SessionMirrorDetailMode.SEND,
            )

            store.set_session_mirror_detail_mode(
                db_path,
                "thread-1",
                SessionMirrorDetailMode.ALL,
            )
            store.upsert_mirror_thread(
                db_path,
                "thread-1",
                "project-1",
                "Renamed",
                100,
                101,
                now=2.0,
            )

            self.assertIs(
                store.get_session_mirror_detail_mode(db_path, "thread-1"),
                SessionMirrorDetailMode.ALL,
            )

    def test_schema_migration_adds_send_mode_to_existing_mirror_rows(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE mirror_threads ("
                    "codex_thread_id TEXT PRIMARY KEY, "
                    "project_key TEXT NOT NULL, "
                    "thread_title TEXT NOT NULL, "
                    "discord_channel_id INTEGER NOT NULL, "
                    "discord_thread_id INTEGER NOT NULL, "
                    "updated_at REAL NOT NULL)"
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-old", "project-1", "Old", 100, 101, 1.0),
                )

            store.init_mirror_db(db_path)

            self.assertIs(
                store.get_session_mirror_detail_mode(db_path, "thread-old"),
                SessionMirrorDetailMode.SEND,
            )

    def test_setting_mode_for_unknown_thread_surfaces_missing_mapping(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"

            self.assertIs(
                store.get_session_mirror_detail_mode(db_path, "missing"),
                SessionMirrorDetailMode.SEND,
            )
            with self.assertRaises(KeyError):
                store.set_session_mirror_detail_mode(
                    db_path,
                    "missing",
                    SessionMirrorDetailMode.ALL,
                )


if __name__ == "__main__":
    _ = unittest.main()

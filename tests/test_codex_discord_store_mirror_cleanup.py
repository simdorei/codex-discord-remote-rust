from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast

import codex_discord_store as store


class StoreMirrorCleanupTests(unittest.TestCase):
    def _db_path(self, temp_dir: str) -> Path:
        db_path = Path(temp_dir) / "mirror.sqlite"
        store.init_mirror_db(db_path)
        return db_path

    def _insert_project(
        self,
        conn: sqlite3.Connection,
        project_key: str,
        project_name: str,
        channel_id: int,
        *,
        updated_at: float = 10.0,
    ) -> None:
        _ = conn.execute(
            "INSERT INTO mirror_projects ("
            + "project_key, project_name, discord_channel_id, updated_at"
            + ") VALUES (?, ?, ?, ?)",
            (project_key, project_name, channel_id, updated_at),
        )

    def _insert_thread(
        self,
        conn: sqlite3.Connection,
        codex_thread_id: str,
        project_key: str,
        thread_title: str,
        channel_id: int,
        thread_id: int,
        *,
        updated_at: float = 20.0,
    ) -> None:
        _ = conn.execute(
            "INSERT INTO mirror_threads ("
            + "codex_thread_id, project_key, thread_title, "
            + "discord_channel_id, discord_thread_id, updated_at"
            + ") VALUES (?, ?, ?, ?, ?, ?)",
            (codex_thread_id, project_key, thread_title, channel_id, thread_id, updated_at),
        )

    def test_stale_mirror_lookups_and_delete_keep_active_rows(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "keep-project", "Keep", 111)
                self._insert_project(conn, "stale-project", "Stale", 222)
                self._insert_thread(conn, "thread-keep", "keep-project", "Keep Thread", 111, 1001)
                self._insert_thread(conn, "thread-stale", "stale-project", "Stale Thread", 222, 1002)

            self.assertEqual(
                store.get_stale_mirror_thread_rows(
                    db_path,
                    {"thread-keep"},
                    updated_before=1000.0,
                ),
                [("thread-stale", 1002, "Stale Thread")],
            )
            self.assertEqual(
                store.get_stale_mirror_project_rows(
                    db_path,
                    {"keep-project"},
                    updated_before=1000.0,
                ),
                [("stale-project", "Stale", 222)],
            )

            store.delete_stale_mirror_rows(
                db_path,
                {"thread-keep"},
                {"keep-project"},
                updated_before=1000.0,
            )
            with sqlite3.connect(db_path) as conn:
                thread_rows = cast(
                    list[tuple[str]],
                    conn.execute(
                        "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id"
                    ).fetchall(),
                )
                project_rows = cast(
                    list[tuple[str]],
                    conn.execute(
                        "SELECT project_key FROM mirror_projects ORDER BY project_key"
                    ).fetchall(),
                )

            self.assertEqual(thread_rows, [("thread-keep",)])
            self.assertEqual(project_rows, [("keep-project",)])

    def test_prune_stale_rows_reports_remaining_discord_ids(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "canonical", "taxlab", 111)
                self._insert_project(conn, "stale-project", "old", 333)
                self._insert_thread(conn, "thread-1", "canonical", "current", 111, 222)
                self._insert_thread(conn, "thread-stale", "stale-project", "old", 333, 444)

            self.assertEqual(
                store.get_stale_mirror_thread_rows(
                    db_path,
                    {"thread-1"},
                    updated_before=1000.0,
                ),
                [("thread-stale", 444, "old")],
            )
            self.assertEqual(
                store.get_stale_mirror_project_rows(
                    db_path,
                    {"canonical"},
                    updated_before=1000.0,
                ),
                [("stale-project", "old", 333)],
            )

            store.delete_stale_mirror_rows(
                db_path,
                {"thread-1"},
                {"canonical"},
                updated_before=1000.0,
            )

            with sqlite3.connect(db_path) as conn:
                project_rows = cast(
                    list[tuple[str]],
                    conn.execute(
                        "SELECT project_key FROM mirror_projects ORDER BY project_key"
                    ).fetchall(),
                )
                thread_rows = cast(
                    list[tuple[str]],
                    conn.execute(
                        "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id"
                    ).fetchall(),
                )

            self.assertEqual(project_rows, [("canonical",)])
            self.assertEqual(thread_rows, [("thread-1",)])
            self.assertEqual(
                store.get_remaining_mirror_discord_ids(db_path),
                ({222}, [111]),
            )

    def test_stale_mirror_delete_with_empty_sets_honors_cutoff(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "old-project", "Old", 111, updated_at=99.0)
                self._insert_project(conn, "equal-project", "Equal", 112, updated_at=100.0)
                self._insert_project(conn, "new-project", "New", 113, updated_at=101.0)
                self._insert_thread(
                    conn, "old-thread", "old-project", "Old", 111, 1001, updated_at=99.0
                )
                self._insert_thread(
                    conn, "equal-thread", "equal-project", "Equal", 112, 1002, updated_at=100.0
                )
                self._insert_thread(
                    conn, "new-thread", "new-project", "New", 113, 1003, updated_at=101.0
                )

            self.assertEqual(
                store.get_stale_mirror_thread_rows(db_path, set(), updated_before=100.0),
                [("old-thread", 1001, "Old")],
            )
            self.assertEqual(
                store.get_stale_mirror_project_rows(db_path, set(), updated_before=100.0),
                [("old-project", "Old", 111)],
            )

            store.delete_stale_mirror_rows(
                db_path,
                set(),
                set(),
                updated_before=100.0,
            )
            with sqlite3.connect(db_path) as conn:
                thread_rows = conn.execute(
                    "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id"
                ).fetchall()
                project_rows = conn.execute(
                    "SELECT project_key FROM mirror_projects ORDER BY project_key"
                ).fetchall()

            self.assertEqual(thread_rows, [("equal-thread",), ("new-thread",)])
            self.assertEqual(project_rows, [("equal-project",), ("new-project",)])

    def test_archive_cleanup_remaining_ids_and_membership(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "project-1", "Project 1", 111)
                self._insert_project(conn, "project-2", "Project 2", 222)
                self._insert_thread(conn, "thread-1", "project-1", "Thread 1", 111, 1001)
                self._insert_thread(conn, "thread-2", "project-2", "Thread 2", 222, 1002)
            store.update_session_mirror_cursor(
                db_path,
                "thread-1",
                "rollout.jsonl",
                7,
                now=30.0,
            )

            self.assertEqual(
                store.get_remaining_mirror_discord_ids(db_path),
                ({1001, 1002}, [111, 222]),
            )
            self.assertFalse(store.is_mirrored_channel_id(db_path, None))
            self.assertTrue(store.is_mirrored_channel_id(db_path, 111))
            self.assertTrue(store.is_mirrored_channel_id(db_path, 1001))
            self.assertFalse(store.is_mirrored_channel_id(db_path, 999))

            self.assertEqual(
                store.delete_archived_mirror_state(db_path, "thread-1"),
                {"mirror_threads": 1, "session_mirror_offsets": 1},
            )
            self.assertEqual(
                store.get_remaining_mirror_discord_ids(db_path),
                ({1002}, [111, 222]),
            )
            self.assertFalse(store.is_mirrored_channel_id(db_path, 1001))
            self.assertTrue(store.is_mirrored_channel_id(db_path, 111))


if __name__ == "__main__":
    _ = unittest.main()

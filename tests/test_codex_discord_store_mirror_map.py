from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex_discord_store as store
import codex_discord_store_mirror_map as store_mirror_map
import codex_discord_store_mirror_threads as store_mirror_threads
import codex_discord_store_startup_probe as store_startup_probe


class StoreMirrorMapTests(unittest.TestCase):
    def test_startup_probe_helpers_keep_store_reexport_and_order_targets(self) -> None:
        self.assertIs(store.get_startup_probe_targets, store_startup_probe.get_startup_probe_targets)
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "project-old", "Old", 20, updated_at=20.0)
                self._insert_project(conn, "project-new", "New", 40, updated_at=30.0)
                self._insert_thread(
                    conn,
                    "thread-startup",
                    "project-new",
                    "Startup duplicate",
                    40,
                    10,
                    updated_at=40.0,
                )
                self._insert_thread(
                    conn,
                    "thread-new",
                    "project-new",
                    "Newest",
                    40,
                    50,
                    updated_at=50.0,
                )

            self.assertEqual(
                store.get_startup_probe_targets(db_path, {30, 20}, 10, limit=5),
                [
                    ("startup", 10),
                    ("allowed", 20),
                    ("allowed", 30),
                    ("mirror_project", 40),
                    ("mirror_thread", 50),
                ],
            )
            self.assertEqual(
                store.get_startup_probe_targets(db_path, {30, 20}, 10, limit=3),
                [
                    ("startup", 10),
                    ("allowed", 20),
                    ("allowed", 30),
                ],
            )

            uninitialized_db_path = Path(temp_dir) / "startup-probe-init.sqlite"
            self.assertEqual(
                store_startup_probe.get_startup_probe_targets(uninitialized_db_path, {7}, None),
                [("allowed", 7)],
            )

    def test_thread_helpers_keep_store_and_map_reexports(self) -> None:
        self.assertIs(store.get_mirrored_codex_thread_id, store_mirror_threads.get_mirrored_codex_thread_id)
        self.assertIs(store.upsert_mirror_thread, store_mirror_threads.upsert_mirror_thread)
        self.assertIs(
            store.get_mirror_thread_row_by_codex_thread_id,
            store_mirror_threads.get_mirror_thread_row_by_codex_thread_id,
        )
        self.assertIs(
            store_mirror_map.get_mirrored_codex_thread_id,
            store_mirror_threads.get_mirrored_codex_thread_id,
        )
        self.assertIs(store_mirror_map.upsert_mirror_thread, store_mirror_threads.upsert_mirror_thread)
        self.assertIs(
            store_mirror_map.get_mirror_thread_row_by_codex_thread_id,
            store_mirror_threads.get_mirror_thread_row_by_codex_thread_id,
        )

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
        updated_at: float,
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
        project_channel_id: int,
        discord_thread_id: int,
        *,
        updated_at: float,
    ) -> None:
        _ = conn.execute(
            "INSERT INTO mirror_threads ("
            + "codex_thread_id, project_key, thread_title, discord_channel_id, "
            + "discord_thread_id, updated_at"
            + ") VALUES (?, ?, ?, ?, ?, ?)",
            (
                codex_thread_id,
                project_key,
                thread_title,
                project_channel_id,
                discord_thread_id,
                updated_at,
            ),
        )

    def test_mirrored_thread_lookup_prefers_direct_and_rejects_ambiguous_project_channel(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_thread(
                    conn,
                    "thread-old",
                    "project",
                    "Old",
                    100,
                    900,
                    updated_at=10.0,
                )

            self.assertIsNone(store.get_mirrored_codex_thread_id(db_path, None))
            self.assertIsNone(store.get_mirrored_codex_thread_id(db_path, 0))
            self.assertEqual(store.get_mirrored_codex_thread_id(db_path, 900), "thread-old")
            self.assertEqual(store.get_mirrored_codex_thread_id(db_path, 100), "thread-old")

            with sqlite3.connect(db_path) as conn:
                self._insert_thread(
                    conn,
                    "thread-new",
                    "project",
                    "New",
                    100,
                    901,
                    updated_at=20.0,
                )

            self.assertEqual(store.get_mirrored_codex_thread_id(db_path, 901), "thread-new")
            self.assertIsNone(store.get_mirrored_codex_thread_id(db_path, 100))

    def test_project_channel_description_and_lookup_shapes(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "project", "Project", 100, updated_at=1.0)

            self.assertEqual(store.describe_mirrored_project_channel(db_path, 100), "")
            self.assertEqual(store.get_mirror_project_for_channel(db_path, 100), ("project", "Project"))
            self.assertIsNone(store.get_mirror_project_for_channel(db_path, 999))
            self.assertIsNone(store.get_mirror_project_for_channel(db_path, 0))

            with sqlite3.connect(db_path) as conn:
                self._insert_thread(
                    conn,
                    "thread-old",
                    "project",
                    "Old",
                    100,
                    900,
                    updated_at=10.0,
                )

            self.assertEqual(store.describe_mirrored_project_channel(db_path, 100), "")

            with sqlite3.connect(db_path) as conn:
                self._insert_thread(
                    conn,
                    "thread-new",
                    "project",
                    "New",
                    100,
                    901,
                    updated_at=20.0,
                )

            self.assertEqual(
                store.describe_mirrored_project_channel(db_path, 100),
                "`Project` project channel has multiple Codex threads.\n"
                + "Send the message inside one of its Discord threads:\n"
                + "- New\n"
                + "- Old",
            )

if __name__ == "__main__":
    _ = unittest.main()

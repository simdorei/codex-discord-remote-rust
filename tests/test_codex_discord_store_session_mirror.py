from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex_discord_store as store


class StoreSessionMirrorTests(unittest.TestCase):
    def _db_path(self, temp_dir: str) -> Path:
        db_path = Path(temp_dir) / "mirror.sqlite"
        store.init_mirror_db(db_path)
        return db_path

    def test_session_mirror_targets_order_and_filter_empty_rows(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                _ = conn.executemany(
                    "INSERT INTO mirror_threads ("
                    + "codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at"
                    + ") VALUES (?, ?, ?, ?, ?, ?)",
                    [
                        ("thread-old", "project", "Old", 10, 100, 1.0),
                        ("", "project", "Missing Codex", 20, 200, 3.0),
                        ("thread-zero-discord", "project", "Zero Discord", 30, 0, 4.0),
                        ("thread-new", "project", "New", 40, 400, 5.0),
                    ],
                )

            self.assertEqual(
                store.get_session_mirror_targets(db_path, limit=10),
                [
                    {
                        "codex_thread_id": "thread-new",
                        "thread_title": "New",
                        "discord_channel_id": 40,
                        "discord_thread_id": 400,
                    },
                    {
                        "codex_thread_id": "thread-old",
                        "thread_title": "Old",
                        "discord_channel_id": 10,
                        "discord_thread_id": 100,
                    },
                ],
            )

    def test_session_mirror_cursor_init_update_and_rollout_reset(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)

            self.assertEqual(
                store.get_or_init_session_mirror_cursor(
                    db_path,
                    "thread-1",
                    "rollout-a.jsonl",
                    5,
                    now=10.0,
                ),
                5,
            )
            self.assertEqual(
                store.get_or_init_session_mirror_cursor(
                    db_path,
                    "thread-1",
                    "rollout-a.jsonl",
                    0,
                    now=11.0,
                ),
                5,
            )

            store.update_session_mirror_cursor(
                db_path,
                "thread-1",
                "rollout-a.jsonl",
                8,
                now=20.0,
            )
            self.assertEqual(
                store.get_session_mirror_offset(db_path, "thread-1"),
                ("rollout-a.jsonl", 8, 20.0),
            )
            self.assertEqual(
                store.get_or_init_session_mirror_cursor(
                    db_path,
                    "thread-1",
                    "rollout-b.jsonl",
                    2,
                    now=30.0,
                ),
                2,
            )
            self.assertEqual(
                store.get_session_mirror_offset(db_path, "thread-1"),
                ("rollout-b.jsonl", 2, 30.0),
            )

    def test_session_mirror_event_claim_lookup_and_cleanup(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)

            self.assertTrue(
                store.claim_session_mirror_event(
                    db_path,
                    "digest-old",
                    "thread-1",
                    now=100.0,
                )
            )
            self.assertFalse(
                store.claim_session_mirror_event(
                    db_path,
                    "digest-old",
                    "thread-1",
                    now=101.0,
                )
            )
            self.assertTrue(
                store.claim_session_mirror_event(
                    db_path,
                    "digest-boundary",
                    "thread-1",
                    now=190.0,
                )
            )
            self.assertTrue(
                store.claim_session_mirror_event(
                    db_path,
                    "digest-new",
                    "thread-1",
                    now=200.0,
                )
            )

            self.assertTrue(
                store.has_session_mirror_event(db_path, "digest-old", "thread-1")
            )
            self.assertEqual(
                store.cleanup_session_mirror_events(
                    db_path,
                    retention_seconds=50.0,
                    now=240.0,
                ),
                1,
            )
            self.assertFalse(
                store.has_session_mirror_event(db_path, "digest-old", "thread-1")
            )
            self.assertTrue(
                store.has_session_mirror_event(db_path, "digest-boundary", "thread-1")
            )
            self.assertTrue(
                store.has_session_mirror_event(db_path, "digest-new", "thread-1")
            )


if __name__ == "__main__":
    _ = unittest.main()

# pyright: reportPrivateUsage=false
from __future__ import annotations

import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

import codex_desktop_bridge_sqlite as bridge_sqlite


class BridgeSqliteTests(unittest.TestCase):
    def test_count_active_threads_counts_unarchived_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            db_path = Path(temp_dir) / "state.sqlite"
            with closing(sqlite3.connect(db_path)) as conn:
                _ = conn.execute("CREATE TABLE threads (archived INTEGER NOT NULL)")
                _ = conn.executemany("INSERT INTO threads (archived) VALUES (?)", [(0,), (1,), (0,)])
                conn.commit()

            self.assertEqual(bridge_sqlite.count_active_threads(db_path), 2)

    def test_first_cell_as_text_handles_missing_rows(self) -> None:
        self.assertIsNone(bridge_sqlite._first_cell_as_text(None))
        self.assertIsNone(bridge_sqlite._first_cell_as_text(()))
        self.assertEqual(bridge_sqlite._first_cell_as_text((3,)), "3")

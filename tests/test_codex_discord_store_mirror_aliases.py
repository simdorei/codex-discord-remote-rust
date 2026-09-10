from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast

import codex_discord_store as store


def _same_project(left: str | None, right: str | None) -> bool:
    return str(left or "").casefold() == str(right or "").casefold()


class StoreMirrorAliasTests(unittest.TestCase):
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

    def test_project_alias_merge_find_and_thread_row_helpers(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "C:/Repo", "Repo Alias", 222, updated_at=5.0)
                self._insert_project(conn, "C:/Other", "Other", 333, updated_at=50.0)
                self._insert_thread(
                    conn,
                    "thread-alias",
                    "C:/Repo",
                    "Alias Thread",
                    222,
                    444,
                    updated_at=6.0,
                )

            aliases = store.upsert_mirror_project(
                db_path,
                "c:/repo",
                "Repo Canon",
                111,
                project_keys_match_func=_same_project,
                now=20.0,
            )
            self.assertEqual(aliases, ["C:/Repo"])

            with sqlite3.connect(db_path) as conn:
                rows = cast(
                    list[tuple[str, str, int]],
                    conn.execute(
                        "SELECT project_key, project_name, discord_channel_id "
                        + "FROM mirror_projects ORDER BY project_key"
                    ).fetchall(),
                )
                thread_project = cast(
                    tuple[str] | None,
                    conn.execute(
                        "SELECT project_key FROM mirror_threads WHERE codex_thread_id = ?",
                        ("thread-alias",),
                    ).fetchone(),
                )
            self.assertEqual(rows, [("C:/Other", "Other", 333), ("c:/repo", "Repo Canon", 111)])
            self.assertEqual(thread_project, ("c:/repo",))

            self.assertEqual(
                store.find_mirror_project_row_by_key(
                    db_path,
                    "c:/repo",
                    project_keys_match_func=_same_project,
                ),
                (111, "c:/repo"),
            )
            self.assertEqual(
                store.find_mirror_project_row_by_key(
                    db_path,
                    "c:/other",
                    project_keys_match_func=_same_project,
                ),
                (333, "C:/Other"),
            )
            self.assertIsNone(
                store.find_mirror_project_row_by_key(
                    db_path,
                    "",
                    project_keys_match_func=_same_project,
                )
            )

            store.upsert_mirror_thread(
                db_path,
                "thread-new",
                "c:/repo",
                "Thread New",
                111,
                555,
                now=30.0,
            )
            self.assertEqual(
                store.get_mirror_thread_row_by_codex_thread_id(db_path, "thread-new"),
                (111, 555),
            )
            self.assertIsNone(store.get_mirror_thread_row_by_codex_thread_id(db_path, "missing"))

    def test_exact_alias_merge_updates_threads_and_read_helpers(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = self._db_path(temp_dir)
            with sqlite3.connect(db_path) as conn:
                self._insert_project(conn, "alias", "taxlab", 111, updated_at=1.0)
                self._insert_thread(
                    conn,
                    "thread-1",
                    "alias",
                    "title",
                    111,
                    222,
                    updated_at=1.0,
                )

            aliases = store.upsert_mirror_project(
                db_path,
                "canonical",
                "taxlab",
                333,
                project_keys_match_func=lambda left, right: left == "alias" and right == "canonical",
                now=2.0,
            )
            store.upsert_mirror_thread(
                db_path,
                "thread-2",
                "canonical",
                "title 2",
                333,
                444,
                now=3.0,
            )

            with sqlite3.connect(db_path) as conn:
                project_rows = conn.execute(
                    "SELECT project_key, discord_channel_id FROM mirror_projects ORDER BY project_key"
                ).fetchall()
                thread_rows = conn.execute(
                    "SELECT codex_thread_id, project_key FROM mirror_threads ORDER BY codex_thread_id"
                ).fetchall()

            self.assertEqual(aliases, ["alias"])
            self.assertEqual(project_rows, [("canonical", 333)])
            self.assertEqual(
                thread_rows,
                [("thread-1", "canonical"), ("thread-2", "canonical")],
            )
            self.assertEqual(
                store.find_mirror_project_row_by_key(
                    db_path,
                    "canonical",
                    project_keys_match_func=lambda left, right: left == right,
                ),
                (333, "canonical"),
            )
            self.assertEqual(
                store.get_mirror_thread_row_by_codex_thread_id(db_path, "thread-2"),
                (333, 444),
            )


if __name__ == "__main__":
    _ = unittest.main()

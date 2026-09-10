from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast

import codex_discord_mirror_channel_store as mirror_store
import codex_discord_store as discord_store
from codex_thread_models import ThreadInfo


def _same_project(left: str | None, right: str | None) -> bool:
    return str(left or "").casefold() == str(right or "").casefold()


class MirrorChannelStoreTests(unittest.TestCase):
    def _deps(self, db_path: Path, logs: list[str] | None = None) -> mirror_store.MirrorChannelDeps:
        log_sink = logs if logs is not None else []
        return mirror_store.MirrorChannelDeps(
            db_path=db_path,
            normalize_project_key=lambda project_key: str(project_key or "").casefold(),
            project_keys_match=_same_project,
            get_thread_ui_name=lambda _thread_id, _thread: "unused",
            log=log_sink.append,
            fetch_failure_types=(LookupError,),
        )

    def test_upsert_project_merges_alias_and_finds_canonical_row(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            discord_store.init_mirror_db(db_path)
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_projects ("
                    + "project_key, project_name, discord_channel_id, updated_at"
                    + ") VALUES (?, ?, ?, ?)",
                    ("C:/Repo", "Old Repo", 111, 1.0),
                )

            logs: list[str] = []
            mirror_store.upsert_mirror_project(
                "c:/repo",
                "Repo",
                222,
                deps=self._deps(db_path, logs),
            )

            self.assertEqual(
                mirror_store.find_mirror_project_row_by_key("C:/REPO", deps=self._deps(db_path)),
                (222, "c:/repo"),
            )
            self.assertEqual(logs, ["mirror_project_aliases_merged project=c:/repo aliases=1"])

    def test_upsert_thread_uses_canonical_project_key(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            discord_store.init_mirror_db(db_path)
            thread = ThreadInfo(
                id="thread-123",
                title="Thread",
                cwd=str(Path("C:/repo")),
                updated_at=1,
                rollout_path="thread.jsonl",
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )

            mirror_store.upsert_mirror_thread(
                thread,
                "C:/Repo",
                "Thread",
                222,
                333,
                deps=self._deps(db_path),
            )

            with sqlite3.connect(db_path) as conn:
                row = cast(
                    tuple[str, int, int] | None,
                    conn.execute(
                        "SELECT project_key, discord_channel_id, discord_thread_id "
                        + "FROM mirror_threads WHERE codex_thread_id = ?",
                        ("thread-123",),
                    ).fetchone(),
                )
            self.assertEqual(row, ("c:/repo", 222, 333))


if __name__ == "__main__":
    _ = unittest.main()

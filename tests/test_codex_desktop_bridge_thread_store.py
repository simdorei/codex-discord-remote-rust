from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import codex_desktop_bridge_state as bridge_state
import codex_desktop_bridge_thread_store as thread_store
import codex_desktop_bridge_thread_store_db as thread_store_db
from codex_thread_models import ThreadInfo


class ThreadStoreErrorTests(unittest.TestCase):
    def test_get_thread_by_id_raises_typed_thread_not_found(self) -> None:
        with self.assertRaises(thread_store.ThreadNotFoundError) as raised:
            _ = thread_store.get_thread_by_id("missing", threads=[])

        self.assertIsInstance(raised.exception, RuntimeError)
        self.assertEqual(str(raised.exception), "Thread not found: missing")
        self.assertEqual(raised.exception.thread_ref, "missing")

    def test_resolve_thread_ref_raises_typed_recent_errors(self) -> None:
        first = _thread("first", "C:\\repo")
        second = _thread("second", "C:\\repo")

        with patch.object(thread_store, "load_recent_threads", return_value=[]):
            with self.assertRaisesRegex(
                thread_store.NoCodexThreadsError,
                "No Codex threads found in the local state DB.",
            ):
                _ = thread_store.resolve_thread_ref("1")

        with (
            patch.object(thread_store, "load_recent_threads", return_value=[first]),
            patch.object(bridge_state, "get_selected_thread_id", return_value="first"),
        ):
            with self.assertRaisesRegex(thread_store.NoAlternateThreadError, "No alternate thread found."):
                _ = thread_store.resolve_thread_ref("other")

        with patch.object(thread_store, "load_recent_threads", return_value=[first]):
            with self.assertRaises(thread_store.ThreadIndexOutOfRangeError) as raised:
                _ = thread_store.resolve_thread_ref("2")
        self.assertEqual(str(raised.exception), "Thread index out of range: 2")
        self.assertEqual(raised.exception.thread_ref, "2")

        with patch.object(thread_store, "load_recent_threads", return_value=[first, second]):
            with self.assertRaises(thread_store.AmbiguousThreadRefError) as ambiguous:
                _ = thread_store.resolve_thread_ref("repo")
        self.assertEqual(
            str(ambiguous.exception),
            "Multiple threads match workspace `repo`. Use one of: repo:1, repo:2",
        )

    def test_resolve_archived_thread_ref_raises_typed_archived_errors(self) -> None:
        first = _thread("archived-1", "C:\\archive")
        second = _thread("archived-2", "C:\\archive")

        with patch.object(thread_store, "load_archived_threads", return_value=[]):
            with self.assertRaisesRegex(
                thread_store.NoArchivedCodexThreadsError,
                "No archived Codex threads found in the local state DB.",
            ):
                _ = thread_store.resolve_archived_thread_ref("1")

        with patch.object(thread_store, "load_archived_threads", return_value=[first]):
            with self.assertRaises(thread_store.ArchivedThreadIndexOutOfRangeError) as raised:
                _ = thread_store.resolve_archived_thread_ref("2")
        self.assertEqual(str(raised.exception), "Archived thread index out of range: 2")
        self.assertEqual(raised.exception.thread_ref, "2")

        with patch.object(thread_store, "load_archived_threads", return_value=[first, second]):
            with self.assertRaises(thread_store.AmbiguousArchivedThreadRefError) as ambiguous:
                _ = thread_store.resolve_archived_thread_ref("archive")
        self.assertEqual(
            str(ambiguous.exception),
            "Multiple archived threads match workspace `archive`. Use one of: archive:1, archive:2",
        )

    def test_choose_thread_raises_typed_errors(self) -> None:
        with patch.object(thread_store, "load_recent_threads", return_value=[]):
            with self.assertRaisesRegex(
                thread_store.NoCodexThreadsError,
                "No Codex threads found in the local state DB.",
            ):
                _ = thread_store.choose_thread(None, None)

        with patch.object(thread_store, "load_recent_threads", return_value=[_thread("thread-1", "C:\\repo")]):
            with self.assertRaises(thread_store.ThreadNotFoundError) as raised:
                _ = thread_store.choose_thread("missing", None)

        self.assertEqual(str(raised.exception), "Thread not found: missing")
        self.assertEqual(raised.exception.thread_ref, "missing")

    def test_choose_thread_finds_explicit_id_outside_recent_window(self) -> None:
        target = _thread("target", "C:\\repo")

        def load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
            if limit == 50:
                return [_thread("recent", "C:\\repo")]
            if limit == 0:
                return [_thread("recent", "C:\\repo"), target]
            raise AssertionError(f"unexpected limit: {limit}")

        with patch.object(thread_store, "load_recent_threads", side_effect=load_recent_threads):
            chosen = thread_store.choose_thread("target", None)

        self.assertEqual(chosen, target)


class ThreadStoreDbScopeTests(unittest.TestCase):
    def test_user_root_scope_excludes_non_codex_chats_and_subagents(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "state.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    """
                    CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        title TEXT,
                        cwd TEXT,
                        updated_at INTEGER,
                        rollout_path TEXT,
                        model TEXT,
                        reasoning_effort TEXT,
                        tokens_used INTEGER,
                        archived INTEGER,
                        source TEXT,
                        thread_source TEXT
                    )
                    """
                )
                _ = conn.executemany(
                    """
                    INSERT INTO threads (
                        id, title, cwd, updated_at, rollout_path, model,
                        reasoning_effort, tokens_used, archived, source, thread_source
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    [
                        _db_thread_row("codex-project", r"C:\repo", "vscode", "user", 4),
                        _db_thread_row(
                            "codex-projectless",
                            r"C:\Users\User\Documents\Codex\2026-07-10\new-chat",
                            "vscode",
                            "",
                            3,
                        ),
                        _db_thread_row("chatgpt", r"C:\repo", "chatgpt", "user", 2),
                        _db_thread_row("codex-subagent", r"C:\repo", "vscode", "subagent", 1),
                    ],
                )

            with patch.object(bridge_state, "STATE_DB_PATH", db_path):
                threads = thread_store_db.load_user_root_threads()

        self.assertEqual([thread.id for thread in threads], ["codex-project", "codex-projectless"])


def _thread(thread_id: str, cwd: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=f"Thread {thread_id}",
        cwd=cwd,
        updated_at=1,
        rollout_path=f"{thread_id}.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


def _db_thread_row(
    thread_id: str,
    cwd: str,
    source: str,
    thread_source: str,
    updated_at: int,
) -> tuple[str, str, str, int, str, str, str, int, int, str, str]:
    return (
        thread_id,
        f"Thread {thread_id}",
        cwd,
        updated_at,
        f"{thread_id}.jsonl",
        "gpt",
        "high",
        0,
        0,
        source,
        thread_source,
    )


if __name__ == "__main__":
    _ = unittest.main()

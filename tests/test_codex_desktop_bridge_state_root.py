from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex_discord_bot as bot
from tests.mirror_sync_bridge_types import bridge_module


class CodexDesktopBridgeStateRootTests(unittest.TestCase):
    def test_load_user_root_threads_reads_db_root_threads_without_subagents(self) -> None:
        bridge = bridge_module()
        old_codex_home = bridge.CODEX_HOME
        old_state_db_path = bridge.STATE_DB_PATH

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                bridge.CODEX_HOME = Path(temp_dir) / "codex-home"
                state_db_path = Path(temp_dir) / "state.sqlite"
                bridge.STATE_DB_PATH = state_db_path
                with sqlite3.connect(state_db_path) as conn:
                    conn.execute(
                        """
                        CREATE TABLE threads (
                            id TEXT PRIMARY KEY,
                            title TEXT NOT NULL,
                            cwd TEXT NOT NULL,
                            updated_at INTEGER NOT NULL,
                            rollout_path TEXT NOT NULL,
                            model TEXT,
                            reasoning_effort TEXT,
                            tokens_used INTEGER NOT NULL DEFAULT 0,
                            archived INTEGER NOT NULL DEFAULT 0,
                            source TEXT NOT NULL,
                            thread_source TEXT
                        )
                        """
                    )
                    conn.executemany(
                        """
                        INSERT INTO threads (
                            id, title, cwd, updated_at, rollout_path, model,
                            reasoning_effort, tokens_used, archived, source, thread_source
                        )
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                        """,
                        [
                            ("root-1", "root", "C:/repo", 3, "root.jsonl", "gpt", "high", 10, 0, "vscode", None),
                            ("sub-1", "", "C:/repo", 4, "sub.jsonl", "gpt", "high", 20, 0, '{"subagent":{}}', "subagent"),
                            ("archived-1", "archived", "C:/repo", 2, "archived.jsonl", "gpt", "high", 30, 1, "vscode", None),
                            ("empty-title", "", "C:/repo", 1, "empty.jsonl", "gpt", "high", 40, 0, "vscode", None),
                        ],
                    )

                threads = bridge.load_user_root_threads(0)

            self.assertEqual([thread.id for thread in threads], ["root-1"])
        finally:
            bridge.CODEX_HOME = old_codex_home
            bridge.STATE_DB_PATH = old_state_db_path

    def test_codex_window_title_filter_rejects_discord_bridge_browser_title(self) -> None:
        bridge = bridge_module()
        self.assertTrue(bridge.is_codex_desktop_window_title("Codex"))
        self.assertTrue(bridge.is_codex_desktop_window_title("Codex - thread"))
        self.assertFalse(
            bridge.is_codex_desktop_window_title(
                'Discord | "taxlab" | Codex app bridge - Chrome'
            )
        )
        self.assertFalse(
            bridge.is_codex_desktop_window_title(
                r"관리자: C:\Users\banpo\AppData\Local\OpenAI\Codex\bin\codex.exe"
            )
        )


if __name__ == "__main__":
    unittest.main()

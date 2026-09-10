from __future__ import annotations

import sqlite3
from pathlib import Path
import tempfile
from typing import override
import unittest

import codex_discord_bot as bot
import codex_discord_project_channels as project_channels


def _init_project_channel_db(db_path: Path) -> None:
    with sqlite3.connect(db_path) as conn:
        _ = conn.execute(
            "CREATE TABLE mirror_threads ("
            + "discord_channel_id INTEGER, project_key TEXT, discord_thread_id INTEGER, updated_at REAL"
            + ")"
        )
        _ = conn.execute(
            "CREATE TABLE mirror_projects ("
            + "discord_channel_id INTEGER, project_key TEXT, updated_at REAL"
            + ")"
        )


class DiscordProjectChannelTests(unittest.TestCase):
    def test_resolve_project_channel_prefers_matching_thread_row(self) -> None:
        init_calls: list[str] = []

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _init_project_channel_db(db_path)
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_threads VALUES (?, ?, ?, ?)",
                    (111, "project-a", 333, 20.0),
                )
                _ = conn.execute(
                    "INSERT INTO mirror_projects VALUES (?, ?, ?)",
                    (333, "project-a", 10.0),
                )

            result = project_channels.resolve_discord_new_thread_project_channel_id(
                333,
                "project-a",
                db_path=db_path,
                init_mirror_db_func=lambda: init_calls.append("init"),
                project_keys_match_func=lambda left, right: left == right,
            )

        self.assertEqual(result, 111)
        self.assertEqual(init_calls, ["init"])

    def test_resolve_project_channel_falls_back_to_matching_project_row(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            _init_project_channel_db(db_path)
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_projects VALUES (?, ?, ?)",
                    (444, "project-b", 10.0),
                )

            result = project_channels.resolve_discord_new_thread_project_channel_id(
                444,
                "project-b",
                db_path=db_path,
                init_mirror_db_func=lambda: None,
                project_keys_match_func=lambda left, right: left == right,
            )

        self.assertEqual(result, 444)


class DiscordBotProjectChannelIntegrationTests(unittest.TestCase):
    _old_db_path: Path = Path()
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_db_path = bot.MIRROR_DB_PATH
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        bot.MIRROR_DB_PATH = Path(temp_dir.name) / "mirror.sqlite"
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_db_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _insert_project(self, project_key: str, channel_id: int) -> None:
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                "INSERT INTO mirror_projects ("
                + "project_key, project_name, discord_channel_id, updated_at"
                + ") VALUES (?, ?, ?, ?)",
                (project_key, "taxlab", channel_id, 1.0),
            )

    def _insert_thread(self, project_key: str, channel_id: int, thread_id: int) -> None:
        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                "INSERT INTO mirror_threads ("
                + "codex_thread_id, project_key, thread_title, "
                + "discord_channel_id, discord_thread_id, updated_at"
                + ") VALUES (?, ?, ?, ?, ?, ?)",
                ("thread-1", project_key, "title", channel_id, thread_id, 1.0),
            )

    def test_new_thread_project_channel_prefers_invoking_thread_parent(self) -> None:
        self._insert_thread(r"c:\taxlab", 111, 222)

        self.assertEqual(
            bot.resolve_discord_new_thread_project_channel_id(222, r"c:\taxlab"),
            111,
        )
        self.assertIsNone(bot.resolve_discord_new_thread_project_channel_id(222, r"c:\other"))

    def test_new_thread_project_channel_matches_normalized_invoking_thread_parent(self) -> None:
        self._insert_thread(r"c:\taxlab", 111, 222)

        self.assertEqual(
            bot.resolve_discord_new_thread_project_channel_id(222, r"\\?\C:\taxlab"),
            111,
        )

    def test_new_thread_project_channel_accepts_project_parent_channel(self) -> None:
        self._insert_project(r"c:\taxlab", 111)

        self.assertEqual(
            bot.resolve_discord_new_thread_project_channel_id(111, r"c:\taxlab"),
            111,
        )


if __name__ == "__main__":
    _ = unittest.main()

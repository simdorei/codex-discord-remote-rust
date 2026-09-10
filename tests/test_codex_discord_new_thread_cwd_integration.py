# pyright: reportAssignmentType=false, reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import sqlite3
import tempfile
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot


@dataclass(frozen=True, slots=True)
class FakeThreadCwd:
    cwd: str


class DiscordNewThreadCwdIntegrationTests(unittest.TestCase):
    def test_prefers_mirrored_thread_cwd(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        original_choose_thread = bridge.choose_thread
        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                expected_cwd = str(Path(temp_dir) / "taxlab")
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    _ = conn.execute(
                        """
                        INSERT INTO mirror_threads (
                            codex_thread_id, project_key, thread_title,
                            discord_channel_id, discord_thread_id, updated_at
                        ) VALUES (?, ?, ?, ?, ?, ?)
                        """,
                        ("thread-1", expected_cwd, "title", 111, 222, 1.0),
                    )
                def fake_choose_thread(thread_id: str, cwd: str | None) -> FakeThreadCwd:
                    _ = (thread_id, cwd)
                    return FakeThreadCwd(expected_cwd)

                bridge.choose_thread = fake_choose_thread

                self.assertEqual(bot.resolve_discord_new_thread_cwd(222), expected_cwd)
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.choose_thread = original_choose_thread

    def test_falls_back_to_project_channel_path(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            project_path = Path(temp_dir) / "project"
            project_path.mkdir()
            bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
            try:
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    _ = conn.execute(
                        """
                        INSERT INTO mirror_projects (
                            project_key, project_name, discord_channel_id, updated_at
                        ) VALUES (?, ?, ?, ?)
                        """,
                        (str(project_path), "project", 333, 1.0),
                    )

                self.assertEqual(bot.resolve_discord_new_thread_cwd(333), str(project_path))
            finally:
                bot.MIRROR_DB_PATH = old_db_path


if __name__ == "__main__":
    _ = unittest.main()

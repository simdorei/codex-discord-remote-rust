from __future__ import annotations

import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

import codex_discord_bot as bot
from codex_thread_models import ThreadInfo
from tests.mirror_sync_bridge_types import bridge_module


class MirrorStatusStateRootTests(unittest.TestCase):
    def make_thread(self, thread_id: str, cwd: str, title: str = "thread") -> ThreadInfo:
        return ThreadInfo(
            id=thread_id,
            title=title,
            cwd=cwd,
            updated_at=1,
            rollout_path=f"{thread_id}.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

    def test_build_mirror_check_defaults_to_state_root_threads(self) -> None:
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
        old_filter_mirrorable_threads = bot.filter_mirrorable_threads
        old_filter_app_server_available_threads = bot.filter_app_server_available_threads
        old_status_builder = bot.discord_mirror_status.build_mirror_check
        root_thread = self.make_thread("root-thread", str(Path.cwd()), "root")
        observed_threads: list[list[ThreadInfo]] = []
        observed_unavailable_counts: list[int] = []

        def fake_load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
            return [root_thread]

        def fake_build_mirror_check(**kwargs) -> str:
            observed_threads.append(kwargs["threads"])
            observed_unavailable_counts.append(kwargs["app_server_unavailable_count"])
            return "Mirror check"

        try:
            bridge.load_user_root_threads = fake_load_user_root_threads
            bot.filter_mirrorable_threads = lambda _threads: (_ for _ in ()).throw(
                AssertionError("mirror check must use the list scope without project filtering")
            )
            bot.filter_app_server_available_threads = lambda threads: list(threads)
            bot.discord_mirror_status.build_mirror_check = fake_build_mirror_check

            output = bot.build_mirror_check()

            self.assertEqual(output, "Mirror check")
            self.assertEqual(observed_threads, [[root_thread]])
            self.assertEqual(observed_unavailable_counts, [0])
        finally:
            bridge.load_user_root_threads = old_load_user_root_threads
            bot.filter_mirrorable_threads = old_filter_mirrorable_threads
            bot.filter_app_server_available_threads = old_filter_app_server_available_threads
            bot.discord_mirror_status.build_mirror_check = old_status_builder

    def test_build_mirror_check_reports_app_server_unavailable_threads(self) -> None:
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
        old_filter_mirrorable_threads = bot.filter_mirrorable_threads
        old_filter_app_server_available_threads = bot.filter_app_server_available_threads
        old_status_builder = bot.discord_mirror_status.build_mirror_check
        available = self.make_thread("available-thread", str(Path.cwd()), "available")
        ghost = self.make_thread("ghost-thread", str(Path.cwd()), "ghost")
        observed_threads: list[list[ThreadInfo]] = []
        observed_unavailable_counts: list[int] = []

        def fake_build_mirror_check(**kwargs) -> str:
            observed_threads.append(kwargs["threads"])
            observed_unavailable_counts.append(kwargs["app_server_unavailable_count"])
            return "Mirror check"

        try:
            bridge.load_user_root_threads = lambda limit=0: [available, ghost]
            bot.filter_mirrorable_threads = lambda threads: list(threads)
            bot.filter_app_server_available_threads = lambda threads: [
                thread for thread in threads if thread.id == available.id
            ]
            bot.discord_mirror_status.build_mirror_check = fake_build_mirror_check

            output = bot.build_mirror_check()

            self.assertEqual(output, "Mirror check")
            self.assertEqual(observed_threads, [[available, ghost]])
            self.assertEqual(observed_unavailable_counts, [1])
        finally:
            bridge.load_user_root_threads = old_load_user_root_threads
            bot.filter_mirrorable_threads = old_filter_mirrorable_threads
            bot.filter_app_server_available_threads = old_filter_app_server_available_threads
            bot.discord_mirror_status.build_mirror_check = old_status_builder

    def test_build_mirror_check_includes_legacy_managed_state_root(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
        old_filter_app_server_available_threads = bot.filter_app_server_available_threads
        old_status_builder = bot.discord_mirror_status.build_mirror_check
        observed_thread_ids: list[list[str]] = []

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    _ = conn.execute(
                        "ALTER TABLE mirror_threads ADD COLUMN managed_by "
                        "TEXT NOT NULL DEFAULT 'ordinary'"
                    )
                    _ = conn.execute(
                        "INSERT INTO mirror_threads "
                        "(codex_thread_id, project_key, thread_title, discord_channel_id, "
                        "discord_thread_id, updated_at, managed_by) "
                        "VALUES (?, ?, ?, ?, ?, ?, ?)",
                        ("managed-root", "legacy:chats", "Legacy", 900, 901, 1.0, "legacy"),
                    )

                ordinary = self.make_thread("ordinary-root", temp_dir)
                managed = self.make_thread("managed-root", temp_dir)
                bridge.load_user_root_threads = lambda limit=0: [ordinary, managed]
                bot.filter_app_server_available_threads = lambda threads: list(threads)

                def fake_build_mirror_check(**kwargs) -> str:
                    observed_thread_ids.append([thread.id for thread in kwargs["threads"]])
                    return "Mirror check"

                bot.discord_mirror_status.build_mirror_check = fake_build_mirror_check

                output = bot.build_mirror_check()

            self.assertEqual(output, "Mirror check")
            self.assertEqual(observed_thread_ids, [["ordinary-root", "managed-root"]])
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.load_user_root_threads = old_load_user_root_threads
            bot.filter_app_server_available_threads = old_filter_app_server_available_threads
            bot.discord_mirror_status.build_mirror_check = old_status_builder

    def test_build_mirror_list_defaults_to_state_root_thread_ids(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
        old_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir) / "discord.log")
                root_thread = self.make_thread("root-thread", str(Path(temp_dir)), "root")
                hidden_thread = self.make_thread("hidden-thread", str(Path(temp_dir)), "hidden")
                bridge.load_user_root_threads = lambda limit=0: [root_thread]
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    conn.execute(
                        "INSERT INTO mirror_projects "
                        "(project_key, project_name, discord_channel_id, updated_at) "
                        "VALUES (?, ?, ?, ?)",
                        (str(Path(temp_dir)), "project", 111, 1.0),
                    )
                    conn.execute(
                        "INSERT INTO mirror_threads "
                        "(codex_thread_id, project_key, thread_title, "
                        "discord_channel_id, discord_thread_id, updated_at) "
                        "VALUES (?, ?, ?, ?, ?, ?)",
                        (hidden_thread.id, str(Path(temp_dir)), "hidden", 111, 222, 3.0),
                    )
                    conn.execute(
                        "INSERT INTO mirror_threads "
                        "(codex_thread_id, project_key, thread_title, "
                        "discord_channel_id, discord_thread_id, updated_at) "
                        "VALUES (?, ?, ?, ?, ?, ?)",
                        (root_thread.id, str(Path(temp_dir)), "root", 111, 333, 1.0),
                    )

                output = bot.build_mirror_list()

            self.assertIn("/ root", output)
            self.assertNotIn("/ hidden", output)
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.load_user_root_threads = old_load_user_root_threads
            if old_log_path is None:
                os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
            else:
                os.environ["CODEX_DISCORD_LOG_PATH"] = old_log_path


if __name__ == "__main__":
    unittest.main()

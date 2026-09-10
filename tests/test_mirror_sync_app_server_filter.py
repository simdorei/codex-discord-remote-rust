from __future__ import annotations

import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from typing import Callable

import codex_discord_bot as bot
from codex_thread_models import ThreadInfo
from tests.mirror_sync_bridge_types import bridge_module, codex_discord_bot


class MirrorSyncAppServerFilterTests(unittest.IsolatedAsyncioTestCase):
    def make_thread(self, thread_id: str, cwd: str, title: str) -> ThreadInfo:
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

    async def test_sync_prunes_db_root_thread_that_app_server_cannot_open(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
        old_filter_mirrorable_threads = bot.filter_mirrorable_threads
        old_filter_app_server_available_threads = getattr(
            bot,
            "filter_app_server_available_threads",
            None,
        )
        old_get_project_channel = bot.get_or_create_project_channel
        old_get_thread_channel = bot.get_or_create_thread_channel
        old_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")

        class FakeGuild:
            def __init__(self) -> None:
                self.categories = [SimpleNamespace(name="Codex", id=999)]
                self.text_channels = []

            def get_channel(self, channel_id: int) -> None:
                return None

            def get_thread(self, thread_id: int) -> None:
                return None

            async def fetch_channel(self, channel_id: int) -> SimpleNamespace:
                return SimpleNamespace(id=channel_id)

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                db_path = Path(temp_dir) / "mirror.sqlite"
                project_path = str(Path(temp_dir))
                bot.MIRROR_DB_PATH = db_path
                os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir) / "discord.log")
                bot.init_mirror_db()
                available = self.make_thread("available-thread", project_path, "available")
                ghost = self.make_thread("ghost-thread", project_path, "ghost")
                with sqlite3.connect(db_path) as conn:
                    conn.execute(
                        "INSERT INTO mirror_projects "
                        "(project_key, project_name, discord_channel_id, updated_at) "
                        "VALUES (?, ?, ?, ?)",
                        (project_path, "project", 111, 1.0),
                    )
                    conn.execute(
                        "INSERT INTO mirror_threads "
                        "(codex_thread_id, project_key, thread_title, "
                        "discord_channel_id, discord_thread_id, updated_at) "
                        "VALUES (?, ?, ?, ?, ?, ?)",
                        (ghost.id, project_path, "ghost", 111, 222, 1.0),
                    )

                def filter_app_server_available_threads(threads: list[ThreadInfo]) -> list[ThreadInfo]:
                    return [thread for thread in threads if thread.id == available.id]

                async def fake_get_project_channel(guild, category, project_key, project_name):
                    bot.upsert_mirror_project(project_key, project_name, 111)
                    return SimpleNamespace(id=111)

                async def fake_get_thread_channel(codex_thread, project_key, project_channel):
                    bot.upsert_mirror_thread(codex_thread, project_key, codex_thread.title, 111, 333)
                    return SimpleNamespace(id=333)

                bridge.load_user_root_threads = lambda limit=0: [available, ghost]
                bot.filter_mirrorable_threads = lambda threads: list(threads)
                setattr(bot, "filter_app_server_available_threads", filter_app_server_available_threads)
                bot.get_or_create_project_channel = fake_get_project_channel
                bot.get_or_create_thread_channel = fake_get_thread_channel
                guild = FakeGuild()
                fake_bot = SimpleNamespace(
                    guild_id=1,
                    guilds=[],
                    user=None,
                    get_guild=lambda guild_id: guild,
                )

                output = await bot.sync_codex_mirror(codex_discord_bot(fake_bot))

                with sqlite3.connect(db_path) as conn:
                    rows = conn.execute(
                        "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id"
                    ).fetchall()

            self.assertIn("threads: 1", output)
            self.assertIn("app_server_unavailable_threads: 1", output)
            self.assertIn("stale_threads_removed: 0", output)
            self.assertEqual(rows, [("available-thread",), ("ghost-thread",)])
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.load_user_root_threads = old_load_user_root_threads
            bot.filter_mirrorable_threads = old_filter_mirrorable_threads
            if old_filter_app_server_available_threads is None:
                if hasattr(bot, "filter_app_server_available_threads"):
                    delattr(bot, "filter_app_server_available_threads")
            else:
                bot.filter_app_server_available_threads = old_filter_app_server_available_threads
            bot.get_or_create_project_channel = old_get_project_channel
            bot.get_or_create_thread_channel = old_get_thread_channel
            if old_log_path is None:
                os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
            else:
                os.environ["CODEX_DISCORD_LOG_PATH"] = old_log_path

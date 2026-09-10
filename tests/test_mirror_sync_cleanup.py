from __future__ import annotations

import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

import discord

import codex_discord_mirror_sync as mirror_sync
import codex_discord_bot as bot
from codex_thread_models import ThreadInfo
from tests.mirror_sync_bridge_types import bridge_module, codex_discord_bot


class MirrorSyncCleanupTests(unittest.IsolatedAsyncioTestCase):
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

    async def test_sync_codex_mirror_removes_db_rows_when_db_root_has_no_active_threads(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        bridge = bridge_module()
        old_load_user_root_threads = bridge.load_user_root_threads
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
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir) / "discord.log")
                bot.init_mirror_db()
                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    conn.execute(
                        "INSERT INTO mirror_projects "
                        "(project_key, project_name, discord_channel_id, updated_at) "
                        "VALUES (?, ?, ?, ?)",
                        ("stale-project", "stale", 111, 1.0),
                    )
                    conn.execute(
                        "INSERT INTO mirror_threads "
                        "(codex_thread_id, project_key, thread_title, "
                        "discord_channel_id, discord_thread_id, updated_at) "
                        "VALUES (?, ?, ?, ?, ?, ?)",
                        ("stale-thread", "stale-project", "stale", 111, 222, 1.0),
                    )

                bridge.load_user_root_threads = lambda limit=0: []
                guild = FakeGuild()
                fake_bot = SimpleNamespace(
                    guild_id=1,
                    guilds=[],
                    user=None,
                    get_guild=lambda guild_id: guild,
                )

                output = await bot.sync_codex_mirror(codex_discord_bot(fake_bot))

                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    thread_rows = conn.execute("SELECT codex_thread_id FROM mirror_threads").fetchall()
                    project_rows = conn.execute("SELECT project_key FROM mirror_projects").fetchall()

            self.assertIn("Mirror sync complete.", output)
            self.assertIn("`rec archive` threads are not removed by sync.", output)
            self.assertIn("Archive those Codex threads first, then run sync.", output)
            self.assertIn("cleanup_scope: full_db_root", output)
            self.assertIn("threads: 0", output)
            self.assertIn("stale_threads_removed: 1", output)
            self.assertEqual(output.count("stale_threads_removed:"), 1)
            self.assertIn("stale_projects_removed: 1", output)
            self.assertEqual(thread_rows, [])
            self.assertEqual(project_rows, [])
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.load_user_root_threads = old_load_user_root_threads
            if old_log_path is None:
                os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
            else:
                os.environ["CODEX_DISCORD_LOG_PATH"] = old_log_path

    async def test_sync_codex_mirror_keeps_rows_outside_requested_limit(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        bridge = bridge_module()
        old_load_recent_threads = bridge.load_recent_threads
        old_filter_mirrorable_threads = bot.filter_mirrorable_threads
        old_filter_app_server_available_threads = bot.filter_app_server_available_threads
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
                bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
                os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir) / "discord.log")
                bot.init_mirror_db()
                scoped_thread = self.make_thread("scoped-thread", str(Path(temp_dir)), "scoped")
                hidden_active_thread = self.make_thread("hidden-active-thread", str(Path(temp_dir)), "hidden")
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
                        (
                            hidden_active_thread.id,
                            str(Path(temp_dir)),
                            "hidden",
                            111,
                            222,
                            1.0,
                        ),
                    )

                load_calls: list[int] = []

                def fake_load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
                    load_calls.append(limit)
                    if limit == 0:
                        return [scoped_thread, hidden_active_thread]
                    return [scoped_thread]

                async def fake_get_project_channel(guild, category, project_key, project_name):
                    bot.upsert_mirror_project(project_key, project_name, 111)
                    return SimpleNamespace(id=111)

                async def fake_get_thread_channel(codex_thread, project_key, project_channel):
                    bot.upsert_mirror_thread(codex_thread, project_key, codex_thread.title, 111, 333)
                    return SimpleNamespace(id=333)

                bridge.load_recent_threads = fake_load_recent_threads
                bot.filter_mirrorable_threads = lambda threads: list(threads)
                bot.filter_app_server_available_threads = lambda threads: list(threads)
                bot.get_or_create_project_channel = fake_get_project_channel
                bot.get_or_create_thread_channel = fake_get_thread_channel
                guild = FakeGuild()
                fake_bot = SimpleNamespace(
                    guild_id=1,
                    guilds=[],
                    user=None,
                    get_guild=lambda guild_id: guild,
                )

                output = await bot.sync_codex_mirror(codex_discord_bot(fake_bot), limit=7)

                with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                    rows = conn.execute(
                        "SELECT codex_thread_id FROM mirror_threads ORDER BY codex_thread_id"
                    ).fetchall()

            self.assertEqual(load_calls, [7])
            self.assertIn("cleanup_scope: limited_sync_no_prune", output)
            self.assertIn("threads: 1", output)
            self.assertIn("stale_threads_removed: 0", output)
            self.assertEqual(rows, [("hidden-active-thread",), ("scoped-thread",)])
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            bridge.load_recent_threads = old_load_recent_threads
            bot.filter_mirrorable_threads = old_filter_mirrorable_threads
            bot.filter_app_server_available_threads = old_filter_app_server_available_threads
            bot.get_or_create_project_channel = old_get_project_channel
            bot.get_or_create_thread_channel = old_get_thread_channel
            if old_log_path is None:
                os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
            else:
                os.environ["CODEX_DISCORD_LOG_PATH"] = old_log_path

    async def test_full_cleanup_does_not_delete_project_channel_shared_by_valid_row(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        old_delete_threads = mirror_sync.discord_mirror_stale.delete_stale_discord_threads
        old_delete_projects = mirror_sync.discord_mirror_stale.delete_stale_project_channels
        old_resolve_projects = mirror_sync.discord_mirror_channels.resolve_orphan_cleanup_project_channels
        old_cleanup_orphans = mirror_sync.discord_mirror_orphans.cleanup_orphan_discord_threads

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                db_path = Path(temp_dir) / "mirror.sqlite"
                bot.MIRROR_DB_PATH = db_path
                bot.init_mirror_db()
                thread = self.make_thread("current-thread", str(Path(temp_dir)), "current")
                with sqlite3.connect(db_path) as conn:
                    conn.execute(
                        "INSERT INTO mirror_projects "
                        "(project_key, project_name, discord_channel_id, updated_at) "
                        "VALUES (?, ?, ?, ?)",
                        ("stale-alias", "project", 111, 1.0),
                    )
                    conn.execute(
                        "INSERT INTO mirror_projects "
                        "(project_key, project_name, discord_channel_id, updated_at) "
                        "VALUES (?, ?, ?, ?)",
                        ("canonical", "project", 111, 2.0),
                    )

                project_cleanup_rows: list[mirror_sync.discord_mirror_stale.StaleMirrorProjectRow] = []

                async def fake_delete_threads(_guild, _stale_rows):
                    return {"deleted": 0, "missing": 0, "failed": 0, "errors": []}

                async def fake_delete_projects(_guild, _category, stale_rows):
                    project_cleanup_rows.extend(stale_rows)
                    return {"deleted": len(stale_rows), "missing": 0, "skipped": 0, "failed": 0, "errors": []}

                async def fake_resolve_project_channels(_guild, _project_channel_ids, *, fetch_failure_types):
                    return []

                async def fake_cleanup_orphan_threads(
                    _project_channels,
                    _known_thread_ids,
                    _bot_user_id,
                    *,
                    is_known_thread_id,
                    delivery_exceptions,
                ):
                    _ = (is_known_thread_id, delivery_exceptions)
                    return {"deleted": 0, "missing": 0, "skipped": 0, "failed": 0, "errors": []}

                mirror_sync.discord_mirror_stale.delete_stale_discord_threads = fake_delete_threads
                mirror_sync.discord_mirror_stale.delete_stale_project_channels = fake_delete_projects
                mirror_sync.discord_mirror_channels.resolve_orphan_cleanup_project_channels = (
                    fake_resolve_project_channels
                )
                mirror_sync.discord_mirror_orphans.cleanup_orphan_discord_threads = fake_cleanup_orphan_threads

                guild = object.__new__(discord.Guild)
                category = object.__new__(discord.CategoryChannel)
                _ = await mirror_sync.cleanup_full_mirror_sync(
                    guild,
                    category,
                    [thread],
                    bot_user_id=None,
                    db_path=db_path,
                    get_project_key=lambda _thread: "canonical",
                    updated_before=1000.0,
                )

                with sqlite3.connect(db_path) as conn:
                    project_rows = conn.execute("SELECT project_key FROM mirror_projects").fetchall()

            self.assertEqual(project_cleanup_rows, [])
            self.assertEqual(project_rows, [("canonical",)])
        finally:
            bot.MIRROR_DB_PATH = old_db_path
            mirror_sync.discord_mirror_stale.delete_stale_discord_threads = old_delete_threads
            mirror_sync.discord_mirror_stale.delete_stale_project_channels = old_delete_projects
            mirror_sync.discord_mirror_channels.resolve_orphan_cleanup_project_channels = old_resolve_projects
            mirror_sync.discord_mirror_orphans.cleanup_orphan_discord_threads = old_cleanup_orphans


if __name__ == "__main__":
    unittest.main()

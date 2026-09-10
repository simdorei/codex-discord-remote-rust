from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import ClassVar

import codex_discord_mirror_status as mirror_status
from codex_thread_models import ThreadContextUsage, ThreadInfo


class FakeMirrorBridge:
    thread: ClassVar[ThreadInfo] = ThreadInfo(
        id="thread-1",
        title="root",
        cwd="C:\\repo",
        updated_at=1,
        rollout_path="thread-1.jsonl",
        model="gpt-5.5",
        reasoning_effort="xhigh",
        tokens_used=0,
    )

    @staticmethod
    def choose_thread(thread_id: str, _cwd: str | None) -> ThreadInfo:
        if thread_id != FakeMirrorBridge.thread.id:
            raise RuntimeError(f"Unknown thread: {thread_id}")  # noqa: GENERIC_ERR_OK
        return FakeMirrorBridge.thread

    @staticmethod
    def get_thread_context_usage(_thread: ThreadInfo) -> ThreadContextUsage | None:
        return None

    @staticmethod
    def describe_thread_context_usage(_context_usage: ThreadContextUsage) -> str:
        return "ok"

    @staticmethod
    def should_recommend_archive(
        _thread: ThreadInfo,
        _context_usage: ThreadContextUsage | None,
    ) -> bool:
        return False

    @staticmethod
    def get_thread_collaboration_mode(_thread: ThreadInfo) -> str:
        return "default"

    @staticmethod
    def get_thread_service_tier(_thread: ThreadInfo) -> str:
        return "fast"

    @staticmethod
    def format_thread_model_display(thread: ThreadInfo, mode: str, speed: str) -> str:
        return f"{thread.model}/{thread.reasoning_effort}/{mode}/{speed}"

    @staticmethod
    def load_recent_threads(limit: int = 20) -> list[ThreadInfo]:
        return [FakeMirrorBridge.thread][:limit]

    @staticmethod
    def get_thread_ui_name(thread_id: str, thread: ThreadInfo | None = None) -> str:
        resolved = thread or FakeMirrorBridge.choose_thread(thread_id, None)
        return resolved.title


class MirrorStatusTests(unittest.TestCase):
    def test_build_mirror_list_includes_model_reasoning_mode_and_speed(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE mirror_projects ("
                    + "project_key TEXT PRIMARY KEY, "
                    + "project_name TEXT NOT NULL, "
                    + "discord_channel_id INTEGER NOT NULL, "
                    + "updated_at REAL NOT NULL"
                    + ")"
                )
                _ = conn.execute(
                    "CREATE TABLE mirror_threads ("
                    + "codex_thread_id TEXT PRIMARY KEY, "
                    + "project_key TEXT NOT NULL, "
                    + "thread_title TEXT NOT NULL, "
                    + "discord_channel_id INTEGER NOT NULL, "
                    + "discord_thread_id INTEGER NOT NULL, "
                    + "updated_at REAL NOT NULL"
                    + ")"
                )
                _ = conn.execute(
                    "INSERT INTO mirror_projects "
                    + "(project_key, project_name, discord_channel_id, updated_at) "
                    + "VALUES (?, ?, ?, ?)",
                    ("project-key", "project", 111, 1.0),
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads "
                    + "(codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at) "
                    + "VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-1", "project-key", "root", 111, 333, 1.0),
                )

            output = mirror_status.build_mirror_list(
                1,
                db_path=db_path,
                init_mirror_db_func=lambda: None,
                bridge_module=FakeMirrorBridge(),
            )

        self.assertIn("model gpt-5.5/xhigh/default/fast", output)
        self.assertIn("discord_thread_id=333", output)
        self.assertIn("parent_channel_id=111", output)
        self.assertIn("accessible=unknown", output)
        self.assertIn("archived=unknown", output)
        self.assertIn("last_seen=1.0", output)
        self.assertIn("stale=false", output)
        self.assertIn("reason=active_mapping", output)

    def test_build_mirror_check_explains_scope_and_sync_cleanup(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE mirror_threads ("
                    + "codex_thread_id TEXT PRIMARY KEY, "
                    + "project_key TEXT NOT NULL, "
                    + "thread_title TEXT NOT NULL, "
                    + "discord_channel_id INTEGER NOT NULL, "
                    + "discord_thread_id INTEGER NOT NULL, "
                    + "updated_at REAL NOT NULL"
                    + ")"
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads "
                    + "(codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at) "
                    + "VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-1", "C:\\repo", "root", 111, 333, 1.0),
                )

            output = mirror_status.build_mirror_check(
                threads=[FakeMirrorBridge.thread],
                db_path=db_path,
                init_mirror_db_func=lambda: None,
                bridge_module=FakeMirrorBridge(),
                filter_mirrorable_threads_func=lambda threads: threads,
                get_project_key_func=lambda _thread: "C:\\repo",
                get_project_name_func=lambda _thread: "repo",
                archive_recommended_count=2,
            )

        self.assertIn("This checks Codex-to-mirror DB mappings only.", output)
        self.assertIn(
            "`!mirror sync` removes stale/orphan threads only under Codex mirror project channels.",
            output,
        )
        self.assertIn("General Discord threads are outside this check.", output)
        self.assertIn("`rec archive` is only a recommendation; archive first, then sync.", output)
        self.assertIn("archive_recommended: 2", output)

    def test_build_mirror_check_can_scope_db_rows_by_project_key(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE mirror_threads ("
                    + "codex_thread_id TEXT PRIMARY KEY, "
                    + "project_key TEXT NOT NULL, "
                    + "thread_title TEXT NOT NULL, "
                    + "discord_channel_id INTEGER NOT NULL, "
                    + "discord_thread_id INTEGER NOT NULL, "
                    + "updated_at REAL NOT NULL"
                    + ")"
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads "
                    + "(codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at) "
                    + "VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-1", "C:\\repo", "root", 111, 333, 2.0),
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads "
                    + "(codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at) "
                    + "VALUES (?, ?, ?, ?, ?, ?)",
                    ("other-thread", "C:\\other", "other", 222, 444, 1.0),
                )

            output = mirror_status.build_mirror_check(
                threads=[FakeMirrorBridge.thread],
                db_path=db_path,
                init_mirror_db_func=lambda: None,
                bridge_module=FakeMirrorBridge(),
                filter_mirrorable_threads_func=lambda threads: threads,
                get_project_key_func=lambda _thread: "C:\\repo",
                get_project_name_func=lambda _thread: "repo",
                scoped_project_keys={"C:\\repo"},
            )

        self.assertIn("codex_threads: 1", output)
        self.assertIn("mirrored_threads: 1", output)
        self.assertIn("stale: 0", output)
        self.assertNotIn("other-thread", output)

    def test_build_mirror_check_stale_rows_include_visibility_fields(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "CREATE TABLE mirror_threads ("
                    + "codex_thread_id TEXT PRIMARY KEY, "
                    + "project_key TEXT NOT NULL, "
                    + "thread_title TEXT NOT NULL, "
                    + "discord_channel_id INTEGER NOT NULL, "
                    + "discord_thread_id INTEGER NOT NULL, "
                    + "updated_at REAL NOT NULL"
                    + ")"
                )
                _ = conn.execute(
                    "INSERT INTO mirror_threads "
                    + "(codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at) "
                    + "VALUES (?, ?, ?, ?, ?, ?)",
                    ("stale-thread", "C:\\repo", "stale", 111, 333, 1.0),
                )

            output = mirror_status.build_mirror_check(
                threads=[],
                db_path=db_path,
                init_mirror_db_func=lambda: None,
                bridge_module=FakeMirrorBridge(),
                filter_mirrorable_threads_func=lambda threads: threads,
                get_project_key_func=lambda _thread: "C:\\repo",
                get_project_name_func=lambda _thread: "repo",
            )

        self.assertIn("Stale:", output)
        self.assertIn("discord_thread_id=333", output)
        self.assertIn("parent_channel_id=111", output)
        self.assertIn("accessible=unknown", output)
        self.assertIn("archived=unknown", output)
        self.assertIn("last_seen=1.0", output)
        self.assertIn("stale=true", output)
        self.assertIn("reason=not_in_active_or_archived_thread_lists", output)

# pyright: reportAssignmentType=false, reportAttributeAccessIssue=false, reportPrivateLocalImportUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false
from __future__ import annotations

import os
import sqlite3
from pathlib import Path
import tempfile
from types import SimpleNamespace
from typing import cast, final, override
import unittest

import codex_desktop_bridge as bridge
import codex_discord_bot as bot
from codex_thread_models import ThreadInfo


class FetchThreadError(RuntimeError):
    pass


class DiscordThreadChannelIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_db_path: Path = Path()
    _old_discord_log_path: str | None = None
    _root: Path = Path()
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_db_path = bot.MIRROR_DB_PATH
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        self._root = Path(temp_dir.name)
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(
            self._root / "discord-smoke.log"
        )
        bot.MIRROR_DB_PATH = self._root / "mirror.sqlite"
        bot.init_mirror_db()

    @override
    def tearDown(self) -> None:
        bot.MIRROR_DB_PATH = self._old_db_path
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _thread_info(self) -> ThreadInfo:
        return ThreadInfo(
            id="thread-1",
            title="Fallback title",
            cwd=str(self._root),
            updated_at=1,
            rollout_path=str(self._root / "thread.jsonl"),
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

    async def test_reuses_existing_thread_by_name(self) -> None:
        original_thread = bot.discord.Thread
        original_get_thread_ui_name = bridge.get_thread_ui_name

        @final
        class FakeThread:
            def __init__(self) -> None:
                self.id = 222
                self.name = "Existing title"

        @final
        class FakeProjectChannel:
            def __init__(self, thread: FakeThread) -> None:
                self.id = 111
                self.threads = [thread]
                self.guild = SimpleNamespace()

            async def create_thread(
                self,
                *_unused_args: str,
                **_unused_kwargs: str | int,
            ) -> FakeThread:
                raise AssertionError("existing mirror thread should be reused")

        def fake_get_thread_ui_name(_thread_id: str, _thread: ThreadInfo | None = None) -> str:
            return "Existing title"

        try:
            bot.discord.Thread = FakeThread
            bridge.get_thread_ui_name = fake_get_thread_ui_name
            existing = FakeThread()

            thread = cast(
                FakeThread,
                await bot.get_or_create_thread_channel(
                    self._thread_info(),
                    r"c:\taxlab",
                    FakeProjectChannel(existing),
                ),
            )

            self.assertIs(thread, existing)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[int, int] | None,
                    conn.execute(
                        """
                        SELECT discord_channel_id, discord_thread_id
                        FROM mirror_threads
                        WHERE codex_thread_id = ?
                        """,
                        ("thread-1",),
                    ).fetchone(),
                )
            self.assertEqual(row, (111, 222))
        finally:
            bot.discord.Thread = original_thread
            bridge.get_thread_ui_name = original_get_thread_ui_name

    async def test_renames_cached_mirror_thread(self) -> None:
        original_thread = bot.discord.Thread
        original_get_thread_ui_name = bridge.get_thread_ui_name

        @final
        class FakeThread:
            def __init__(self) -> None:
                self.id = 222
                self.parent_id = 111
                self.name = "old title"
                self.edits: list[dict[str, str]] = []

            async def edit(self, **kwargs: str) -> None:
                self.edits.append(kwargs)
                self.name = str(kwargs.get("name", self.name))

        @final
        class FakeGuild:
            def __init__(self, thread: FakeThread) -> None:
                self.thread = thread

            def get_thread(self, thread_id: int) -> FakeThread | None:
                return self.thread if thread_id == 222 else None

        @final
        class FakeProjectChannel:
            def __init__(self, thread: FakeThread) -> None:
                self.id = 111
                self.threads: list[FakeThread] = []
                self.guild = FakeGuild(thread)

            async def create_thread(
                self,
                *_unused_args: str,
                **_unused_kwargs: str | int,
            ) -> FakeThread:
                raise AssertionError("cached mirror thread should be reused")

        def fake_get_thread_ui_name(_thread_id: str, _thread: ThreadInfo | None = None) -> str:
            return "Current title"

        try:
            bot.discord.Thread = FakeThread
            bridge.get_thread_ui_name = fake_get_thread_ui_name
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_threads ("
                    + "codex_thread_id, project_key, thread_title, "
                    + "discord_channel_id, discord_thread_id, updated_at"
                    + ") VALUES (?, ?, ?, ?, ?, ?)",
                    ("thread-1", r"c:\taxlab", "old title", 111, 222, 1.0),
                )
            existing = FakeThread()

            thread = cast(
                FakeThread,
                await bot.get_or_create_thread_channel(
                    self._thread_info(),
                    r"c:\taxlab",
                    FakeProjectChannel(existing),
                ),
            )

            self.assertIs(thread, existing)
            self.assertEqual(existing.name, "Current title")
            self.assertEqual(len(existing.edits), 1)
            with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
                row = cast(
                    tuple[str, int, int] | None,
                    conn.execute(
                        """
                        SELECT thread_title, discord_channel_id, discord_thread_id
                        FROM mirror_threads
                        WHERE codex_thread_id = ?
                        """,
                        ("thread-1",),
                    ).fetchone(),
                )
            self.assertEqual(row, ("Current title", 111, 222))
        finally:
            bot.discord.Thread = original_thread
            bridge.get_thread_ui_name = original_get_thread_ui_name

    async def test_surfaces_missing_cached_thread(self) -> None:
        @final
        class FakeGuild:
            def get_thread(self, _thread_id: int) -> None:
                return None

            async def fetch_channel(self, _thread_id: int) -> None:
                raise FetchThreadError("thread boom")

        @final
        class FakeProjectChannel:
            def __init__(self) -> None:
                self.id = 111
                self.threads: list[str] = []
                self.guild = FakeGuild()

            async def create_thread(
                self,
                *_unused_args: str,
                **_unused_kwargs: str | int,
            ) -> None:
                raise AssertionError("missing cached thread should not create a replacement")

        with sqlite3.connect(bot.MIRROR_DB_PATH) as conn:
            _ = conn.execute(
                "INSERT INTO mirror_threads ("
                + "codex_thread_id, project_key, thread_title, "
                + "discord_channel_id, discord_thread_id, updated_at"
                + ") VALUES (?, ?, ?, ?, ?, ?)",
                ("thread-1", r"c:\taxlab", "old title", 111, 222, 1.0),
            )

        with self.assertRaisesRegex(
            RuntimeError,
            r"Stored mirror thread 222 .*FetchThreadError: thread boom",
        ):
            await bot.get_or_create_thread_channel(
                self._thread_info(),
                r"c:\taxlab",
                FakeProjectChannel(),
            )


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

import sqlite3
import tempfile
import unittest
from collections.abc import AsyncIterator, Sequence
from pathlib import Path
from unittest import mock

import discord

import codex_discord_mirror_stale as mirror_stale
import codex_discord_mirror_sync as mirror_sync
import codex_discord_mirror_sync_result as mirror_result
import codex_discord_store as store
from codex_thread_models import ThreadInfo


class FakeCategory(discord.CategoryChannel):
    pass


class FakeGuild(discord.Guild):
    pass


class FakeDiscordThread:
    def __init__(self, thread_id: int) -> None:
        self.id = thread_id
        self.owner_id = 123
        self.name = "Concurrent Thread"
        self.deleted = False

    async def delete(self, *, reason: str) -> None:
        self.deleted = bool(reason)


class FakeProjectChannel:
    def __init__(self, thread: FakeDiscordThread) -> None:
        self.threads = [thread]
        self.name = "codex-project"

    async def archived_threads(self, *, limit: int) -> AsyncIterator[FakeDiscordThread]:
        _ = limit
        for thread in self.threads[:0]:
            yield thread


class MirrorSyncOrphanSnapshotTests(unittest.IsolatedAsyncioTestCase):
    async def test_orphan_cleanup_keeps_mapping_created_after_known_id_snapshot(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            store.init_mirror_db(db_path)
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_projects VALUES (?, ?, ?, ?)",
                    ("active-project", "Active", 900, 1.0),
                )

            active_thread = ThreadInfo(
                id="active-thread",
                title="Active",
                cwd=temp_dir,
                updated_at=1,
                rollout_path="active.jsonl",
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )
            discord_thread = FakeDiscordThread(901)
            project_channel = FakeProjectChannel(discord_thread)

            async def insert_mapping_after_snapshot(
                guild: FakeGuild,
                stale_rows: Sequence[mirror_stale.StaleMirrorThreadRow],
            ) -> mirror_result.MirrorCleanupResult:
                _ = (guild, stale_rows)
                with sqlite3.connect(db_path) as conn:
                    _ = conn.execute(
                        "INSERT INTO mirror_threads VALUES (?, ?, ?, ?, ?, ?)",
                        (
                            "created-after-known-snapshot",
                            "active-project",
                            "Concurrent Thread",
                            900,
                            901,
                            101.0,
                        ),
                    )
                return {"deleted": 0, "missing": 0, "failed": 0, "errors": []}

            async def keep_projects(
                guild: FakeGuild,
                category: FakeCategory,
                stale_rows: Sequence[mirror_stale.StaleMirrorProjectRow],
            ) -> mirror_result.MirrorCleanupResult:
                _ = (guild, category, stale_rows)
                return {"deleted": 0, "missing": 0, "skipped": 0, "failed": 0, "errors": []}

            async def resolve_projects(
                guild: FakeGuild,
                project_channel_ids: list[int],
                *,
                fetch_failure_types: tuple[type[Exception], ...],
            ) -> list[FakeProjectChannel]:
                _ = (guild, project_channel_ids, fetch_failure_types)
                return [project_channel]

            with (
                mock.patch.object(
                    mirror_sync.discord_mirror_stale,
                    "delete_stale_discord_threads",
                    insert_mapping_after_snapshot,
                ),
                mock.patch.object(
                    mirror_sync.discord_mirror_stale,
                    "delete_stale_project_channels",
                    keep_projects,
                ),
                mock.patch.object(
                    mirror_sync.discord_mirror_channels,
                    "resolve_orphan_cleanup_project_channels",
                    resolve_projects,
                ),
            ):
                _ = await mirror_sync.cleanup_full_mirror_sync(
                    object.__new__(FakeGuild),
                    object.__new__(FakeCategory),
                    [active_thread],
                    bot_user_id=123,
                    db_path=db_path,
                    get_project_key=lambda thread: "active-project",
                    updated_before=100.0,
                )

            with sqlite3.connect(db_path) as conn:
                concurrent_row = conn.execute(
                    "SELECT codex_thread_id FROM mirror_threads WHERE discord_thread_id = 901"
                ).fetchone()

        self.assertEqual(concurrent_row, ("created-after-known-snapshot",))
        self.assertFalse(discord_thread.deleted)


if __name__ == "__main__":
    _ = unittest.main()

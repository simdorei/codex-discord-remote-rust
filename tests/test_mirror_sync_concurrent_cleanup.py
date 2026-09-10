from __future__ import annotations

import sqlite3
import tempfile
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from unittest import mock

import codex_discord_mirror_sync as mirror_sync
import codex_discord_store as store
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class FakeCategory:
    id: int


class FakeGuild:
    def get_channel(self, channel_id: int) -> None:
        _ = channel_id
        return None

    def get_thread(self, thread_id: int) -> None:
        _ = thread_id
        return None

    async def fetch_channel(self, channel_id: int) -> None:
        _ = channel_id
        return None


class MirrorSyncConcurrentCleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_full_sync_keeps_mapping_created_after_scope_snapshot(self) -> None:
        # Given: full sync already captured an active scope without a newly created mapping.
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            store.init_mirror_db(db_path)
            clock = {"now": 100.0}
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "ALTER TABLE mirror_threads ADD COLUMN managed_by "
                    + "TEXT NOT NULL DEFAULT 'ordinary'"
                )
                _ = conn.execute(
                    "ALTER TABLE mirror_threads ADD COLUMN lifecycle_state "
                    + "TEXT NOT NULL DEFAULT 'active'"
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
            guild = FakeGuild()
            category = FakeCategory(id=999)

            async def get_mirror_guild(bot: str) -> FakeGuild:
                _ = bot
                return guild

            async def get_mirror_category(current_guild: FakeGuild) -> FakeCategory:
                _ = current_guild
                return category

            async def get_project_channel(
                current_guild: FakeGuild,
                current_category: FakeCategory,
                project_key: str,
                project_name: str,
            ) -> str:
                _ = (current_guild, current_category, project_key, project_name)
                return "active-project-channel"

            async def get_thread_channel(
                codex_thread: ThreadInfo,
                project_key: str,
                project_channel: str,
            ) -> str:
                _ = (codex_thread, project_key, project_channel)
                with sqlite3.connect(db_path) as conn:
                    _ = conn.execute(
                        "INSERT INTO mirror_projects VALUES (?, ?, ?, ?)",
                        ("concurrent-project", "Concurrent", 900, 150.0),
                    )
                    _ = conn.execute(
                        "INSERT INTO mirror_threads VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                        (
                            "created-during-sync",
                            "concurrent-project",
                            "Concurrent Thread",
                            900,
                            901,
                            150.0,
                            "ordinary",
                            "active",
                        ),
                    )
                clock["now"] = 200.0
                return "active-thread-channel"

            deps: mirror_sync.CodexMirrorSyncDeps[
                str,
                ThreadInfo,
                FakeGuild,
                FakeCategory,
                str,
                str,
            ]
            deps = mirror_sync.CodexMirrorSyncDeps(
                db_path=db_path,
                get_mirror_guild=get_mirror_guild,
                get_or_create_mirror_category=get_mirror_category,
                load_mirror_scope_threads=lambda limit: [active_thread],
                filter_mirrorable_threads=lambda threads: list(threads),
                filter_app_server_available_threads=lambda threads: list(threads),
                get_project_key=lambda thread: "active-project",
                get_project_name=lambda thread: "Active Project",
                get_or_create_project_channel=get_project_channel,
                get_or_create_thread_channel=get_thread_channel,
                get_bot_user_id=lambda bot: None,
                log=lambda message: None,
            )

            # When: awaited mirror creation adds the mapping before stale cleanup runs.
            with mock.patch.object(
                mirror_sync.time, "time", side_effect=lambda: clock["now"]
            ):
                _ = await mirror_sync.sync_codex_mirror("bot", deps=deps)

            # Then: this sync generation must not delete the mapping it did not initially observe.
            with sqlite3.connect(db_path) as conn:
                concurrent_row = cast(
                    tuple[str] | None,
                    conn.execute(
                        "SELECT codex_thread_id FROM mirror_threads "
                        + "WHERE codex_thread_id = 'created-during-sync'"
                    ).fetchone(),
                )
                concurrent_project_row = cast(
                    tuple[str] | None,
                    conn.execute(
                        "SELECT project_key FROM mirror_projects WHERE project_key = 'concurrent-project'"
                    ).fetchone(),
                )

        self.assertEqual(concurrent_row, ("created-during-sync",))
        self.assertEqual(concurrent_project_row, ("concurrent-project",))


if __name__ == "__main__":
    _ = unittest.main()

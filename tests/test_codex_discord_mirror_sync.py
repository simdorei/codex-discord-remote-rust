from __future__ import annotations

import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import override
from unittest import mock

import discord

import codex_discord_mirror_sync as mirror_sync


@dataclass(frozen=True, slots=True)
class FakeThread:
    id: str
    project_key: str
    project_name: str


@dataclass(frozen=True, slots=True)
class FakeProjectChannel:
    key: str


@dataclass(frozen=True, slots=True)
class FakeThreadChannel:
    id: str


class MirrorSyncTests(unittest.IsolatedAsyncioTestCase):
    async def test_ensure_mirror_threads_reuses_projects_and_reverses_threads(self) -> None:
        events: list[tuple[str, str, str]] = []
        threads = [
            FakeThread("thread-1", "project-a", "Project A"),
            FakeThread("thread-2", "project-a", "Project A"),
            FakeThread("thread-3", "project-b", "Project B"),
        ]

        async def get_project_channel(
            guild: str,
            category: str,
            project_key: str,
            project_name: str,
        ) -> FakeProjectChannel:
            events.append((f"project:{guild}:{category}", project_key, project_name))
            return FakeProjectChannel(project_key)

        async def get_thread_channel(
            codex_thread: FakeThread,
            project_key: str,
            project_channel: FakeProjectChannel,
        ) -> FakeThreadChannel:
            events.append((f"thread:{project_channel.key}", project_key, codex_thread.id))
            return FakeThreadChannel(codex_thread.id)

        result = await mirror_sync.ensure_mirror_threads(
            "guild",
            "category",
            threads,
            deps=mirror_sync.MirrorThreadEnsureDeps(
                lambda thread: thread.project_key,
                lambda thread: thread.project_name,
                get_project_channel,
                get_thread_channel,
            ),
        )

        self.assertEqual(result.mirrored, 3)
        self.assertEqual(set(result.projects), {"project-a", "project-b"})
        self.assertEqual(
            events,
            [
                ("project:guild:category", "project-b", "Project B"),
                ("thread:project-b", "project-b", "thread-3"),
                ("project:guild:category", "project-a", "Project A"),
                ("thread:project-a", "project-a", "thread-2"),
                ("thread:project-a", "project-a", "thread-1"),
            ],
        )

    async def test_cleanup_full_mirror_sync_uses_valid_scope_and_orphan_cleanup(self) -> None:
        class FakeGuild(discord.Guild):
            @override
            def __str__(self) -> str:
                return "guild"

        class FakeCategory(discord.CategoryChannel):
            @override
            def __str__(self) -> str:
                return "category"

        guild = object.__new__(FakeGuild)
        category = object.__new__(FakeCategory)
        cleanup_full_mirror_sync = mirror_sync.cleanup_full_mirror_sync
        events: list[tuple[str, str]] = []
        threads = [
            FakeThread("thread-1", "project-a", "Project A"),
            FakeThread("thread-2", "project-b", "Project B"),
        ]
        db_path = Path("mirror.sqlite")

        async def delete_stale_threads(guild, stale_rows):
            events.append(("delete_stale_threads", str(guild)))
            self.assertEqual(stale_rows, [("stale-thread", 222, "Stale")])
            return {"deleted": 1, "missing": 0, "failed": 0, "errors": []}

        async def delete_stale_projects(guild, category, stale_rows):
            events.append(("delete_stale_projects", f"{guild}:{category}"))
            self.assertEqual(stale_rows, [("stale-project", "Stale Project", 333)])
            return {"deleted": 1, "missing": 0, "skipped": 0, "failed": 0, "errors": []}

        async def resolve_project_channels(guild, project_channel_ids, *, fetch_failure_types):
            _ = fetch_failure_types
            events.append(("resolve_project_channels", f"{guild}:{list(project_channel_ids)}"))
            return ["project-channel"]

        async def cleanup_orphans(
            project_channels,
            known_thread_ids,
            bot_user_id,
            *,
            is_known_thread_id,
            delivery_exceptions,
        ):
            _ = (is_known_thread_id, delivery_exceptions)
            events.append(("cleanup_orphans", f"{list(project_channels)}:{sorted(known_thread_ids)}:{bot_user_id}"))
            return {"deleted": 2, "skipped": 3, "failed": 0, "errors": []}

        with (
            mock.patch.object(
                mirror_sync.discord_store,
                "get_stale_mirror_thread_rows",
                return_value=[("stale-thread", 222, "Stale")],
            ) as get_stale_threads,
            mock.patch.object(
                mirror_sync.discord_store,
                "get_stale_mirror_project_rows",
                return_value=[("stale-project", "Stale Project", 333)],
            ) as get_stale_projects,
            mock.patch.object(mirror_sync.discord_mirror_stale, "delete_stale_discord_threads", delete_stale_threads),
            mock.patch.object(mirror_sync.discord_mirror_stale, "delete_stale_project_channels", delete_stale_projects),
            mock.patch.object(mirror_sync.discord_store, "delete_stale_mirror_rows") as delete_rows,
            mock.patch.object(
                mirror_sync.discord_store,
                "get_remaining_mirror_discord_ids",
                return_value=({444}, [555]),
            ) as get_remaining,
            mock.patch.object(
                mirror_sync.discord_mirror_channels,
                "resolve_orphan_cleanup_project_channels",
                resolve_project_channels,
            ),
            mock.patch.object(mirror_sync.discord_mirror_orphans, "cleanup_orphan_discord_threads", cleanup_orphans),
        ):
            result = await cleanup_full_mirror_sync(
                guild,
                category,
                threads,
                bot_user_id=999,
                db_path=db_path,
                get_project_key=lambda thread: thread.project_key,
                updated_before=30.0,
            )

        get_stale_threads.assert_called_once_with(
            db_path,
            {"thread-1", "thread-2"},
            updated_before=30.0,
        )
        get_stale_projects.assert_called_once_with(
            db_path,
            {"project-a", "project-b"},
            updated_before=30.0,
        )
        delete_rows.assert_called_once_with(
            db_path,
            {"thread-1", "thread-2"},
            {"project-a", "project-b"},
            updated_before=30.0,
        )
        get_remaining.assert_called_once_with(db_path)
        self.assertEqual(len(result.stale_threads), 1)
        self.assertEqual(len(result.stale_projects), 1)
        self.assertEqual(result.stale_cleanup["deleted"], 1)
        self.assertEqual(result.stale_project_cleanup["deleted"], 1)
        self.assertEqual(result.orphan_cleanup["deleted"], 2)
        self.assertEqual(
            events,
            [
                ("delete_stale_threads", "guild"),
                ("delete_stale_projects", "guild:category"),
                ("resolve_project_channels", "guild:[555]"),
                ("cleanup_orphans", "['project-channel']:[444]:999"),
            ],
        )


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import sqlite3
import tempfile
import unittest
from collections.abc import AsyncIterator, Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import cast
from unittest import mock

import discord

import codex_discord_mirror_single_thread as mirror_single
import codex_discord_mirror_sync as mirror_sync
import codex_discord_store as store
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class FakeCategory:
    id: int


class FakeGuild:
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


class MirrorSyncMutationCoordinationTests(unittest.IsolatedAsyncioTestCase):
    async def test_orphan_delete_serializes_single_thread_mirroring(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            store.init_mirror_db(db_path)
            with sqlite3.connect(db_path) as conn:
                _ = conn.execute(
                    "INSERT INTO mirror_projects VALUES (?, ?, ?, ?)",
                    ("active-project", "Active", 900, 100.0),
                )

            delete_started = asyncio.Event()
            allow_delete = asyncio.Event()
            single_started = asyncio.Event()

            class BlockingDiscordThread(FakeDiscordThread):
                async def delete(self, *, reason: str) -> None:
                    _ = reason
                    delete_started.set()
                    await allow_delete.wait()
                    self.deleted = True

            project_channel = FakeProjectChannel(BlockingDiscordThread(901))

            async def get_single_guild(bot: str) -> FakeGuild:
                _ = bot
                single_started.set()
                return FakeGuild()

            async def get_sync_guild(bot: str) -> FakeGuild:
                _ = bot
                return FakeGuild()

            async def get_category(guild: FakeGuild) -> FakeCategory:
                _ = guild
                return FakeCategory(id=999)

            single_codex_thread = ThreadInfo(
                id="single-thread",
                title="Single",
                cwd=temp_dir,
                updated_at=1,
                rollout_path="single.jsonl",
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )

            async def get_project_channel(
                guild: FakeGuild,
                category: FakeCategory,
                project_key: str,
                project_name: str,
            ) -> FakeProjectChannel:
                _ = (guild, category, project_key, project_name)
                return project_channel

            async def get_thread_channel(
                codex_thread: ThreadInfo,
                project_key: str,
                channel: FakeProjectChannel,
            ) -> FakeDiscordThread:
                _ = (codex_thread, project_key, channel)
                return FakeDiscordThread(902)

            sync_deps: mirror_sync.CodexMirrorSyncDeps[
                str,
                ThreadInfo,
                FakeGuild,
                FakeCategory,
                FakeProjectChannel,
                FakeDiscordThread,
            ]
            sync_deps = mirror_sync.CodexMirrorSyncDeps(
                db_path=db_path,
                get_mirror_guild=get_sync_guild,
                get_or_create_mirror_category=get_category,
                load_mirror_scope_threads=lambda limit: [],
                filter_mirrorable_threads=lambda threads: list(threads),
                filter_app_server_available_threads=lambda threads: list(threads),
                get_project_key=lambda thread: "active-project",
                get_project_name=lambda thread: "Active",
                get_or_create_project_channel=get_project_channel,
                get_or_create_thread_channel=get_thread_channel,
                get_bot_user_id=lambda bot: 123,
                log=lambda message: None,
            )
            single_deps: mirror_single.MirrorSingleThreadDeps[str]
            single_deps = mirror_single.MirrorSingleThreadDeps(
                get_mirror_guild=cast(
                    Callable[[str], Awaitable[discord.Guild]],
                    get_single_guild,
                ),
                get_or_create_mirror_category=cast(
                    Callable[[discord.Guild], Awaitable[discord.CategoryChannel]],
                    get_category,
                ),
                choose_thread=lambda thread_id, fallback: single_codex_thread,
                get_project_key=lambda thread: "active-project",
                get_project_name=lambda thread: "Active",
                upsert_mirror_project=lambda project_key, project_name, channel_id: (
                    None
                ),
                get_or_create_project_channel=cast(
                    Callable[
                        [discord.Guild, discord.CategoryChannel, str, str],
                        Awaitable[discord.TextChannel],
                    ],
                    get_project_channel,
                ),
                get_or_create_thread_channel=cast(
                    Callable[
                        [ThreadInfo, str, discord.TextChannel],
                        Awaitable[discord.Thread],
                    ],
                    get_thread_channel,
                ),
                delivery_exceptions=(RuntimeError,),
                log=lambda message: None,
            )

            async def resolve_projects(
                guild: FakeGuild,
                project_channel_ids: list[int],
                *,
                fetch_failure_types: tuple[type[Exception], ...],
            ) -> list[FakeProjectChannel]:
                _ = (guild, project_channel_ids, fetch_failure_types)
                return [project_channel]

            with (
                mock.patch.object(mirror_sync.time, "time", return_value=100.0),
                mock.patch.object(
                    mirror_sync.discord_mirror_channels,
                    "resolve_orphan_cleanup_project_channels",
                    resolve_projects,
                ),
            ):
                sync_task = asyncio.ensure_future(
                    mirror_sync.sync_codex_mirror("bot", deps=sync_deps)
                )
                await delete_started.wait()
                single_task = asyncio.ensure_future(
                    mirror_single.mirror_single_codex_thread(
                        "bot",
                        "single-thread",
                        deps=single_deps,
                    )
                )
                await asyncio.sleep(0)
                was_serialized = not single_started.is_set()
                allow_delete.set()
                _ = await sync_task
                _ = await single_task

        self.assertTrue(was_serialized)


if __name__ == "__main__":
    _ = unittest.main()

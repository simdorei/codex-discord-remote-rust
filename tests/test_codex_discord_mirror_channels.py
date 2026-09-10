from __future__ import annotations

import unittest
from pathlib import Path
from typing import cast

import discord

import codex_discord_mirror_channels as mirror_channels
import codex_discord_mirror_project_channels as mirror_project_channels
import codex_discord_mirror_thread_channels as mirror_thread_channels
import codex_discord_mirror_thread_store as mirror_thread_store
from codex_thread_models import ThreadInfo


class MirrorChannelHelperTests(unittest.TestCase):
    def test_project_channel_helpers_are_reexported_from_mirror_channels(self) -> None:
        self.assertIs(
            mirror_channels.ensure_mirror_project_channel,
            mirror_project_channels.ensure_mirror_project_channel,
        )
        self.assertIs(
            mirror_channels.get_or_create_project_channel,
            mirror_project_channels.get_or_create_project_channel,
        )

    def test_find_existing_project_channel_prefers_mirror_topic(self) -> None:
        original_text_channel = discord.TextChannel

        class FakeTextChannel:
            def __init__(self, name: str, topic: str) -> None:
                self.id: int = 111
                self.name: str = name
                self.topic: str = topic

        class FakeGuild:
            def __init__(self, channels: list[FakeTextChannel]) -> None:
                self.text_channels: list[FakeTextChannel] = channels

        try:
            discord.TextChannel = FakeTextChannel
            plain_name_match = FakeTextChannel("codex-taxlab", "")
            topic_match = FakeTextChannel("codex-other", "Codex project mirror: taxlab")

            channel = mirror_channels.find_existing_project_channel(
                FakeGuild([plain_name_match, topic_match]),
                project_name="taxlab",
                base_name="codex-taxlab",
            )

            self.assertIs(channel, topic_match)
        finally:
            discord.TextChannel = original_text_channel

    def test_get_mirror_thread_name_uses_ui_title(self) -> None:
        thread = ThreadInfo(
            id="thread-12345678",
            title="Fallback title",
            cwd=str(Path("C:/repo")),
            updated_at=1,
            rollout_path="thread.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )
        deps = mirror_channels.MirrorChannelDeps(
            db_path=Path("mirror.sqlite"),
            normalize_project_key=lambda project_key: str(project_key or "").lower(),
            project_keys_match=lambda left, right: left == right,
            get_thread_ui_name=lambda _thread_id, _thread: "UI title",
            log=lambda _message: None,
            fetch_failure_types=(RuntimeError,),
        )

        self.assertEqual(
            mirror_thread_channels.get_mirror_thread_name(thread, deps=deps),
            "UI title",
        )


class MirrorThreadChannelHelperTests(unittest.IsolatedAsyncioTestCase):
    def _deps(self, *fetch_failure_types: type[Exception]) -> mirror_channels.MirrorChannelDeps:
        return mirror_channels.MirrorChannelDeps(
            db_path=Path("mirror.sqlite"),
            normalize_project_key=lambda project_key: str(project_key or "").lower(),
            project_keys_match=lambda left, right: left == right,
            get_thread_ui_name=lambda _thread_id, _thread: "unused",
            log=lambda _message: None,
            fetch_failure_types=fetch_failure_types or (RuntimeError,),
        )

    def _thread_info(self) -> ThreadInfo:
        return ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd=str(Path("C:/repo")),
            updated_at=1,
            rollout_path="thread.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

    def test_stored_mirror_thread_ids_rejects_wrong_project_channel(self) -> None:
        class FakeTextChannel:
            id: int = 222

        with self.assertRaisesRegex(RuntimeError, "belongs to project channel 111"):
            _ = mirror_thread_store.stored_mirror_thread_ids(
                (111, 333),
                self._thread_info(),
                cast(discord.TextChannel, cast(object, FakeTextChannel())),
            )

    async def test_fetch_stored_discord_thread_prefers_cache_and_fetches_missing(self) -> None:
        original_thread = discord.Thread

        class FakeThread:
            def __init__(self, thread_id: int) -> None:
                self.id: int = thread_id
                self.parent_id: int = 222

        class FakeGuild:
            def __init__(self) -> None:
                self.cached: FakeThread = FakeThread(333)
                self.fetched: FakeThread = FakeThread(444)
                self.fetch_calls: list[int] = []

            def get_thread(self, thread_id: int) -> FakeThread | None:
                if thread_id == self.cached.id:
                    return self.cached
                return None

            async def fetch_channel(self, thread_id: int) -> FakeThread:
                self.fetch_calls.append(thread_id)
                return self.fetched

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id: int = 222
                self.guild: FakeGuild = FakeGuild()

        try:
            discord.Thread = FakeThread
            channel = FakeTextChannel()
            cached = await mirror_thread_store.fetch_stored_discord_thread(
                self._thread_info(),
                cast(discord.TextChannel, cast(object, channel)),
                333,
                deps=self._deps(),
            )
            fetched = await mirror_thread_store.fetch_stored_discord_thread(
                self._thread_info(),
                cast(discord.TextChannel, cast(object, channel)),
                444,
                deps=self._deps(),
            )
        finally:
            discord.Thread = original_thread

        self.assertIs(cached, channel.guild.cached)
        self.assertIs(fetched, channel.guild.fetched)
        self.assertEqual(channel.guild.fetch_calls, [444])

    async def test_fetch_stored_discord_thread_rejects_thread_from_other_project(self) -> None:
        original_thread = discord.Thread

        class FakeThread:
            def __init__(self) -> None:
                self.id: int = 333
                self.parent_id: int = 999

        class FakeGuild:
            def get_thread(self, _thread_id: int) -> FakeThread:
                return FakeThread()

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id: int = 222
                self.guild: FakeGuild = FakeGuild()

        try:
            discord.Thread = FakeThread
            with self.assertRaisesRegex(RuntimeError, "belongs to Discord channel 999"):
                _ = await mirror_thread_store.fetch_stored_discord_thread(
                    self._thread_info(),
                    cast(discord.TextChannel, cast(object, FakeTextChannel())),
                    333,
                    deps=self._deps(),
                )
        finally:
            discord.Thread = original_thread

    async def test_fetch_stored_discord_thread_wraps_fetch_failures(self) -> None:
        original_thread = discord.Thread

        class FakeFetchError(RuntimeError):
            pass

        class FakeThread:
            id: int = 333

        class FakeGuild:
            def get_thread(self, _thread_id: int) -> None:
                return None

            async def fetch_channel(self, _thread_id: int) -> FakeThread:
                raise FakeFetchError("gone")

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id: int = 222
                self.guild: FakeGuild = FakeGuild()

        try:
            discord.Thread = FakeThread
            with self.assertRaisesRegex(RuntimeError, "is unavailable: FakeFetchError: gone"):
                _ = await mirror_thread_store.fetch_stored_discord_thread(
                    self._thread_info(),
                    cast(discord.TextChannel, cast(object, FakeTextChannel())),
                    333,
                    deps=self._deps(FakeFetchError),
                )
        finally:
            discord.Thread = original_thread

    async def test_find_existing_thread_channel_logs_fetch_failure_when_archive_scan_raises(
        self,
    ) -> None:
        class FakeArchiveScanError(RuntimeError):
            pass

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id: int = 222
                self.threads: list[str] = []
                self.archived_scan_limit: int | None = None

            def archived_threads(self, *, limit: int) -> None:
                self.archived_scan_limit = limit
                raise FakeArchiveScanError("scan boom")

        logs: list[str] = []
        deps = mirror_channels.MirrorChannelDeps(
            db_path=Path("mirror.sqlite"),
            normalize_project_key=lambda project_key: str(project_key or "").lower(),
            project_keys_match=lambda left, right: left == right,
            get_thread_ui_name=lambda _thread_id, _thread: "unused",
            log=logs.append,
            fetch_failure_types=(FakeArchiveScanError,),
        )
        project_channel = FakeTextChannel()

        result = await mirror_thread_channels.find_existing_thread_channel(
            cast(discord.TextChannel, cast(object, project_channel)),
            "missing-thread",
            deps=deps,
        )

        self.assertIsNone(result)
        self.assertEqual(project_channel.archived_scan_limit, 100)
        self.assertEqual(
            logs,
            ["mirror_thread_reuse_scan_failed channel=222 error=scan boom"],
        )

if __name__ == "__main__":
    _ = unittest.main()

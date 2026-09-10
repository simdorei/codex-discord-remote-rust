from __future__ import annotations

import sqlite3
import tempfile
import unittest
from pathlib import Path
from typing import cast

import discord

import codex_discord_mirror_channels as mirror_channels
import codex_discord_mirror_thread_channels as mirror_thread_channels
from codex_thread_models import ThreadInfo


class MirrorThreadReuseOwnershipTests(unittest.IsolatedAsyncioTestCase):
    async def test_creates_thread_when_name_match_is_owned_by_another_codex_thread(self) -> None:
        original_thread = discord.Thread
        logs: list[str] = []
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            db_path = Path(temp_dir) / "mirror.sqlite"
            deps = _deps(db_path, logs)
            mirror_channels.upsert_mirror_thread(
                _thread_info("thread-other"),
                "Project",
                "unused",
                222,
                444,
                deps=deps,
            )
            project_channel = FakeTextChannel(FakeThread(444, "unused"))

            try:
                discord.Thread = FakeThread
                result = await mirror_thread_channels.get_or_create_thread_channel(
                    _thread_info("thread-1"),
                    "Project",
                    cast(discord.TextChannel, cast(object, project_channel)),
                    deps=deps,
                )
            finally:
                discord.Thread = original_thread

            with sqlite3.connect(db_path) as conn:
                rows = conn.execute(
                    "SELECT codex_thread_id, discord_thread_id FROM mirror_threads ORDER BY codex_thread_id",
                ).fetchall()

        self.assertEqual(result.id, 555)
        self.assertEqual(project_channel.created_threads, ["unused"])
        self.assertEqual(rows, [("thread-1", 555), ("thread-other", 444)])
        self.assertIn("mirror_thread_reuse_skipped", "\n".join(logs))


class FakeThread:
    def __init__(self, thread_id: int, name: str) -> None:
        self.id: int = thread_id
        self.name: str = name

    async def edit(self, *, name: str, reason: str) -> "FakeThread":
        _ = reason
        self.name = name
        return self


class FakeTextChannel:
    def __init__(self, thread: FakeThread) -> None:
        self.id: int = 222
        self.threads: list[FakeThread] = [thread]
        self.created_threads: list[str] = []

    async def create_thread(
        self,
        *,
        name: str,
        type: discord.ChannelType,
        auto_archive_duration: int,
        reason: str,
    ) -> FakeThread:
        _ = type, auto_archive_duration, reason
        self.created_threads.append(name)
        return FakeThread(555, name)


def _deps(db_path: Path, logs: list[str]) -> mirror_channels.MirrorChannelDeps:
    return mirror_channels.MirrorChannelDeps(
        db_path=db_path,
        normalize_project_key=lambda project_key: str(project_key or "").lower(),
        project_keys_match=lambda left, right: left == right,
        get_thread_ui_name=lambda _thread_id, _thread: "unused",
        log=logs.append,
        fetch_failure_types=(RuntimeError,),
    )


def _thread_info(thread_id: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title="Thread",
        cwd=str(Path("C:/repo")),
        updated_at=1,
        rollout_path=f"{thread_id}.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


if __name__ == "__main__":
    _ = unittest.main()

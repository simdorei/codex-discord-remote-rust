from __future__ import annotations

import unittest

import codex_discord_session_mirror_commit as mirror_commit


class SessionMirrorCommitTests(unittest.IsolatedAsyncioTestCase):
    async def test_terminal_sent_deactivates_updates_cursor_and_logs_summary(self) -> None:
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        operations: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))
            operations.append("cursor")

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            operations.append("release")
            return True

        await mirror_commit.commit_session_mirror_delivery(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=2,
            sent_count=1,
            terminal_sent=True,
            deps=mirror_commit.SessionMirrorCommitDeps(
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=logs.append,
            ),
        )

        self.assertEqual(released, ["thread-1"])
        self.assertEqual(operations, ["cursor", "release"])
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
        self.assertEqual(
            logs,
            ["session_mirror_sent target=thread-1 channel=333 events=2 items=1 cursor=42"],
        )

    async def test_no_sent_items_updates_cursor_without_summary_log(self) -> None:
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            return True

        await mirror_commit.commit_session_mirror_delivery(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=2,
            sent_count=0,
            terminal_sent=False,
            deps=mirror_commit.SessionMirrorCommitDeps(
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=logs.append,
            ),
        )

        self.assertEqual(released, [])
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
        self.assertEqual(logs, [])

    async def test_nonterminal_sent_item_keeps_target_active_and_logs_summary(self) -> None:
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            return True

        await mirror_commit.commit_session_mirror_delivery(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=3,
            sent_count=2,
            terminal_sent=False,
            deps=mirror_commit.SessionMirrorCommitDeps(
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=logs.append,
            ),
        )

        self.assertEqual(released, [])
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
        self.assertEqual(
            logs,
            ["session_mirror_sent target=thread-1 channel=333 events=3 items=2 cursor=42"],
        )

    async def test_terminal_without_sent_items_deactivates_without_summary_log(self) -> None:
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        logs: list[str] = []

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            return True

        await mirror_commit.commit_session_mirror_delivery(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=2,
            sent_count=0,
            terminal_sent=True,
            deps=mirror_commit.SessionMirrorCommitDeps(
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=logs.append,
            ),
        )

        self.assertEqual(released, ["thread-1"])
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
        self.assertEqual(logs, [])

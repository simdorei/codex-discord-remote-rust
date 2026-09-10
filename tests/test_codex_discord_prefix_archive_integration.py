from __future__ import annotations

# pyright: reportAttributeAccessIssue=false, reportUnknownMemberType=false, reportUnknownVariableType=false
import unittest

import codex_discord_bot as bot
import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo

from tests.test_codex_discord_bot import FakeBot, FakeMessage, FakeTarget


class DiscordPrefixArchiveIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def test_prefix_archive_named_ref_keeps_thread_ref_resolver(self) -> None:
        original_run_bridge_and_send = bot.run_bridge_and_send
        original_load_user_root_threads = bridge.load_user_root_threads
        original_resolve_thread_ref = bridge.resolve_thread_ref
        calls: list[tuple[list[str], str]] = []
        thread = ThreadInfo(
            id="workspace-thread",
            title="workspace",
            cwd=r"C:\repo",
            updated_at=1,
            rollout_path="workspace.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

        async def fake_run_bridge_and_send(
            target: FakeTarget,
            argv: list[str],
            title: str,
            failure_title: str | None = None,
            archive_cleanup_owner: bot.CodexDiscordBot | None = None,
        ) -> tuple[int, str]:
            _ = (failure_title, archive_cleanup_owner)
            calls.append((argv, title))
            await target.send("ok")
            return 0, "ok"

        def fail_load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
            _ = limit
            raise AssertionError("archive named ref should not load DB-root index")

        def fake_resolve_thread_ref(ref: str) -> ThreadInfo:
            _ = ref
            return thread

        try:
            bot.run_bridge_and_send = fake_run_bridge_and_send
            bridge.load_user_root_threads = fail_load_user_root_threads
            bridge.resolve_thread_ref = fake_resolve_thread_ref

            message = FakeMessage(channel_id=333)
            await bot.handle_prefix_command(FakeBot(), message, "archive taxlab:1")

            self.assertEqual(calls, [(["archive", "--thread-id", "workspace-thread"], "Archive")])
            self.assertEqual(message.channel.messages, [("ok", None)])
        finally:
            bot.run_bridge_and_send = original_run_bridge_and_send
            bridge.load_user_root_threads = original_load_user_root_threads
            bridge.resolve_thread_ref = original_resolve_thread_ref

    async def test_prefix_archive_numeric_ref_uses_db_root_order(self) -> None:
        original_run_bridge_and_send = bot.run_bridge_and_send
        original_load_user_root_threads = bridge.load_user_root_threads
        original_resolve_thread_ref = bridge.resolve_thread_ref
        calls: list[tuple[list[str], str]] = []

        def make_thread(thread_id: str, updated_at: int) -> ThreadInfo:
            return ThreadInfo(
                id=thread_id,
                title=thread_id,
                cwd=r"C:\repo",
                updated_at=updated_at,
                rollout_path=f"{thread_id}.jsonl",
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )

        async def fake_run_bridge_and_send(
            target: FakeTarget,
            argv: list[str],
            title: str,
            failure_title: str | None = None,
            archive_cleanup_owner: bot.CodexDiscordBot | None = None,
        ) -> tuple[int, str]:
            _ = (failure_title, archive_cleanup_owner)
            calls.append((argv, title))
            await target.send("ok")
            return 0, "ok"

        def fake_load_user_root_threads(limit: int = 0) -> list[ThreadInfo]:
            _ = limit
            return [
                make_thread("root-1", 3),
                make_thread("root-2", 2),
                make_thread("root-3", 1),
            ]

        def fail_recent_resolver(ref: str) -> ThreadInfo:
            raise AssertionError(f"archive numeric ref should use DB-root order, got {ref}")

        try:
            bot.run_bridge_and_send = fake_run_bridge_and_send
            bridge.load_user_root_threads = fake_load_user_root_threads
            bridge.resolve_thread_ref = fail_recent_resolver

            message = FakeMessage(channel_id=333)
            await bot.handle_prefix_command(FakeBot(), message, "archive 2")

            self.assertEqual(calls, [(["archive", "--thread-id", "root-2"], "Archive")])
            self.assertEqual(message.channel.messages, [("ok", None)])
        finally:
            bot.run_bridge_and_send = original_run_bridge_and_send
            bridge.load_user_root_threads = original_load_user_root_threads
            bridge.resolve_thread_ref = original_resolve_thread_ref

    async def test_archive_list_alias_routes_to_archived_list(self) -> None:
        original_run_bridge_and_send = bot.run_bridge_and_send
        calls: list[tuple[list[str], str]] = []

        async def fake_run_bridge_and_send(
            target: FakeTarget,
            argv: list[str],
            title: str,
            failure_title: str | None = None,
            archive_cleanup_owner: bot.CodexDiscordBot | None = None,
        ) -> tuple[int, str]:
            _ = (failure_title, archive_cleanup_owner)
            calls.append((argv, title))
            await target.send("ok")
            return 0, "ok"

        try:
            bot.run_bridge_and_send = fake_run_bridge_and_send
            message = FakeMessage()
            await bot.handle_prefix_command(FakeBot(), message, "archive_list 5")

            self.assertEqual(calls, [(["archived_list", "--limit", "5"], "Archived list")])
            self.assertEqual(message.channel.messages, [("ok", None)])
        finally:
            bot.run_bridge_and_send = original_run_bridge_and_send


if __name__ == "__main__":
    _ = unittest.main()

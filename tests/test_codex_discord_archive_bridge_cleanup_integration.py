from __future__ import annotations

from pathlib import Path
import unittest

import codex_discord_bot as bot
import codex_discord_store as discord_store

from tests.codex_discord_archive_cleanup_test_helpers import (
    ArchiveBridgeCleanupTestCase,
    ArchiveCleanupOwnerFake,
    CleanupUnavailable,
    EmptyArchiveCleanupOwner,
    bridge_command_result,
    fetch_archive_rows,
    insert_archive_mirror_state,
    run_archive_bridge_and_send,
)
from tests.test_codex_discord_bot import FakeTarget


class DiscordArchiveBridgeCleanupIntegrationTests(ArchiveBridgeCleanupTestCase):
    async def test_archive_bridge_success_cleans_exact_mirror_state(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        try:
            insert_archive_mirror_state(include_project=True, include_event=True)
            bot.activate_pending_session_mirror_output_target("thread-1")
            owner = ArchiveCleanupOwnerFake()
            bot.run_bridge_command = bridge_command_result(
                0,
                "archived_thread: thread-1\ntitle: Archived",
            )
            target = FakeTarget()

            exit_code, output = await run_archive_bridge_and_send()(
                target,
                ["archive", "--thread-id", "thread-1"],
                "Archive",
                archive_cleanup_owner=owner,
            )
            rows = fetch_archive_rows()

            self.assertEqual(exit_code, 0)
            self.assertEqual(output, "archived_thread: thread-1\ntitle: Archived")
            self.assertEqual(target.messages, [("Archive\n\narchived_thread: thread-1\ntitle: Archived", None)])
            self.assertEqual(rows.mirror_rows, [])
            self.assertEqual(rows.offset_rows, [])
            self.assertEqual(rows.project_rows, [("project",)])
            self.assertEqual(rows.event_rows, [("digest-1",)])
            self.assertFalse(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertFalse(bot.is_pending_session_mirror_cursor_target("thread-1"))
            self.assertEqual(owner.skip_logged, set())
            self.assertEqual(owner.seen_agent_messages, {})
            self.assertEqual(owner.seen_user_messages, {})
            log_text = self.log_text()
            self.assertIn("archive_mirror_cleanup_done target=thread-1", log_text)
            self.assertIn("mirror_rows=1", log_text)
            self.assertIn("offsets=1", log_text)
        finally:
            bot.run_bridge_command = original_run_bridge_command

    async def test_archive_bridge_failure_does_not_cleanup_mirror_state(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        try:
            insert_archive_mirror_state()
            bot.activate_session_mirror_output_target("thread-1")
            owner = ArchiveCleanupOwnerFake()
            bot.run_bridge_command = bridge_command_result(1, "archive failed")
            target = FakeTarget()

            _ = await run_archive_bridge_and_send()(
                target,
                ["archive", "--thread-id", "thread-1"],
                "Archive",
                archive_cleanup_owner=owner,
            )
            rows = fetch_archive_rows()

            self.assertEqual(target.messages, [("Archive failed (exit 1)\n\narchive failed", None)])
            self.assertEqual(rows.mirror_rows, [("thread-1",)])
            self.assertEqual(rows.offset_rows, [("thread-1",)])
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertEqual(owner.skip_logged, {"thread-1"})
            self.assertEqual(owner.seen_agent_messages, {"thread-1": {"agent": 1.0}})
            self.assertEqual(owner.seen_user_messages, {"thread-1": {"user": 1.0}})
        finally:
            bot.run_bridge_command = original_run_bridge_command

    async def test_archive_bridge_success_without_archived_thread_skips_cleanup(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        try:
            insert_archive_mirror_state()
            bot.activate_pending_session_mirror_output_target("thread-1")
            owner = ArchiveCleanupOwnerFake()
            bot.run_bridge_command = bridge_command_result(0, "title: Archived")
            target = FakeTarget()

            _ = await run_archive_bridge_and_send()(
                target,
                ["archive", "--thread-id", "thread-1"],
                "Archive",
                archive_cleanup_owner=owner,
            )
            rows = fetch_archive_rows()

            self.assertEqual(target.messages, [("Archive\n\ntitle: Archived", None)])
            self.assertEqual(rows.mirror_rows, [("thread-1",)])
            self.assertEqual(rows.offset_rows, [("thread-1",)])
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertTrue(bot.is_pending_session_mirror_cursor_target("thread-1"))
            self.assertEqual(owner.skip_logged, {"thread-1"})
            self.assertIn("archive_mirror_cleanup_skipped reason=no_archived_thread", self.log_text())
        finally:
            bot.run_bridge_command = original_run_bridge_command

    async def test_archive_bridge_cleanup_failure_warns_without_blocking_response(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        original_delete_archived_mirror_state = discord_store.delete_archived_mirror_state
        try:
            def fail_cleanup(db_path: Path, codex_thread_id: str) -> dict[str, int]:
                _ = db_path, codex_thread_id
                raise CleanupUnavailable("cleanup unavailable")

            bot.run_bridge_command = bridge_command_result(0, "archived_thread: thread-1")
            discord_store.delete_archived_mirror_state = fail_cleanup
            target = FakeTarget()

            exit_code, output = await run_archive_bridge_and_send()(
                target,
                ["archive", "--thread-id", "thread-1"],
                "Archive",
                archive_cleanup_owner=EmptyArchiveCleanupOwner(),
            )

            self.assertEqual(exit_code, 0)
            self.assertEqual(output, "archived_thread: thread-1")
            self.assertEqual(
                target.messages,
                [
                    (
                        (
                            "Archive\n\narchived_thread: thread-1\n\n"
                            + "Mirror cleanup warning: CleanupUnavailable: cleanup unavailable"
                        ),
                        None,
                    )
                ],
            )
            log_text = self.log_text()
            self.assertIn("archive_mirror_cleanup_failed target=thread-1 error_type=CleanupUnavailable", log_text)
            self.assertIn("CleanupUnavailable: cleanup unavailable", log_text)
        finally:
            bot.run_bridge_command = original_run_bridge_command
            discord_store.delete_archived_mirror_state = original_delete_archived_mirror_state


if __name__ == "__main__":
    _ = unittest.main()

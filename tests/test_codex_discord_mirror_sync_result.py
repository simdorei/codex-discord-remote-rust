from __future__ import annotations

import unittest
from pathlib import Path

import codex_discord_mirror_sync_result as sync_result


class MirrorSyncResultTests(unittest.TestCase):
    def test_empty_mirror_cleanup_result_keeps_existing_dict_shape(self) -> None:
        result = sync_result.empty_mirror_cleanup_result()

        self.assertEqual(result["deleted"], 0)
        self.assertEqual(result["missing"], 0)
        self.assertEqual(result["skipped"], 0)
        self.assertEqual(result["failed"], 0)
        self.assertEqual(result["errors"], [])

    def test_format_mirror_sync_result_preserves_policy_and_cleanup_lines(self) -> None:
        output = sync_result.format_mirror_sync_result(
            cleanup_scope="full_db_root",
            project_count=2,
            mirrored=7,
            stale_thread_count=1,
            stale_project_count=1,
            stale_cleanup={
                "deleted": 1,
                "missing": 2,
                "skipped": 0,
                "failed": 3,
                "errors": ["stale failed"],
            },
            orphan_cleanup={
                "deleted": 4,
                "missing": 0,
                "skipped": 5,
                "failed": 6,
                "errors": ["orphan failed"],
            },
            stale_project_cleanup={
                "deleted": 7,
                "missing": 8,
                "skipped": 9,
                "failed": 10,
                "errors": ["project failed"],
            },
            db_path=Path("mirror.sqlite"),
        )

        self.assertIn("Mirror sync complete.", output)
        self.assertIn("`rec archive` threads are not removed by sync.", output)
        self.assertIn("cleanup_scope: full_db_root", output)
        self.assertIn("projects: 2", output)
        self.assertIn("threads: 7", output)
        self.assertIn("stale_threads_removed: 1", output)
        self.assertIn("stale_discord_threads_deleted: 1", output)
        self.assertIn("orphan_discord_threads_skipped: 5", output)
        self.assertIn("stale_project_channels_failed: 10", output)
        self.assertIn("Discord stale cleanup errors:", output)
        self.assertIn("- stale failed", output)
        self.assertIn("Discord orphan cleanup errors:", output)
        self.assertIn("- orphan failed", output)
        self.assertIn("Discord stale project cleanup errors:", output)
        self.assertIn("- project failed", output)


if __name__ == "__main__":
    _ = unittest.main()

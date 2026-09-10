from __future__ import annotations

import unittest

import codex_discord_ready_cleanup as ready_cleanup


class CleanupRuntimeError(RuntimeError):
    pass


class CleanupTypeError(TypeError):
    pass


class DiscordReadyCleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_logs_deleted_count_when_cleanup_returns_count(self) -> None:
        logs: list[str] = []

        await ready_cleanup.run_ready_cleanup(
            lambda: 3,
            deleted_event="cleanup_deleted",
            failed_event="cleanup_failed",
            log=logs.append,
        )

        self.assertEqual(logs, ["cleanup_deleted count=3"])

    async def test_logs_runtime_failure_and_continues(self) -> None:
        logs: list[str] = []

        def fail_cleanup() -> int:
            raise CleanupRuntimeError("cleanup unavailable")

        await ready_cleanup.run_ready_cleanup(
            fail_cleanup,
            deleted_event="cleanup_deleted",
            failed_event="cleanup_failed",
            log=logs.append,
        )

        self.assertTrue(any("cleanup_failed" in line for line in logs))

    async def test_type_error_is_not_cleanup_failure(self) -> None:
        logs: list[str] = []

        def fail_cleanup() -> int:
            raise CleanupTypeError("bad cleanup dependency")

        with self.assertRaisesRegex(TypeError, "bad cleanup dependency"):
            await ready_cleanup.run_ready_cleanup(
                fail_cleanup,
                deleted_event="cleanup_deleted",
                failed_event="cleanup_failed",
                log=logs.append,
            )

        self.assertEqual(logs, [])


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from dataclasses import dataclass
import unittest

import codex_discord_archive_targets as archive_targets


@dataclass(frozen=True, slots=True)
class FakeThread:
    id: str


class FakeBridge:
    def __init__(self, threads: list[FakeThread]) -> None:
        self._threads: tuple[FakeThread, ...] = tuple(threads)

    def load_user_root_threads(self) -> tuple[FakeThread, ...]:
        return self._threads


class DiscordArchiveTargetTests(unittest.TestCase):
    def test_numeric_ref_resolves_against_db_root_threads(self) -> None:
        def fallback(channel_id: int | None, ref: str | None) -> list[str]:
            raise AssertionError(f"unexpected fallback {channel_id=} {ref=}")

        self.assertEqual(
            archive_targets.resolve_discord_archive_target_args(
                222,
                "2",
                bridge_module=FakeBridge([FakeThread("thread-1"), FakeThread("thread-2")]),
                resolve_thread_target_args_func=fallback,
            ),
            ["--thread-id", "thread-2"],
        )

    def test_out_of_range_numeric_ref_reports_db_root_index_error(self) -> None:
        def fallback(channel_id: int | None, ref: str | None) -> list[str]:
            raise AssertionError(f"unexpected fallback {channel_id=} {ref=}")

        with self.assertRaisesRegex(RuntimeError, "DB root thread index out of range: 3"):
            _ = archive_targets.resolve_discord_archive_target_args(
                222,
                "3",
                bridge_module=FakeBridge([FakeThread("thread-1")]),
                resolve_thread_target_args_func=fallback,
            )

    def test_non_numeric_ref_uses_existing_thread_target_resolver(self) -> None:
        calls: list[tuple[int | None, str | None]] = []

        def fallback(channel_id: int | None, ref: str | None) -> list[str]:
            calls.append((channel_id, ref))
            return ["--thread-id", "fallback-thread"]

        self.assertEqual(
            archive_targets.resolve_discord_archive_target_args(
                222,
                "taxlab:1",
                bridge_module=FakeBridge([]),
                resolve_thread_target_args_func=fallback,
            ),
            ["--thread-id", "fallback-thread"],
        )
        self.assertEqual(calls, [(222, "taxlab:1")])


if __name__ == "__main__":
    _ = unittest.main()

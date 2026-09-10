from __future__ import annotations

from dataclasses import dataclass, field
import unittest

import codex_discord_session_mirror_archive as archive_policy


@dataclass(slots=True)
class FakeMirrorState:
    active_output_targets: dict[str, float] = field(
        default_factory=lambda: {"thread-1": 1.0}
    )
    pending_cursor_targets: set[str] = field(default_factory=lambda: {"thread-1"})


class SessionMirrorArchivePolicyTests(unittest.TestCase):
    def test_archive_recommended_without_active_output_logs_once_and_tails_only(self) -> None:
        skip_logged: set[str] = set()
        logs: list[str] = []

        first_tail_only = archive_policy.resolve_session_mirror_archive_policy(
            "thread-1",
            archive_recommended=True,
            active_output_target=False,
            archive_skip_logged=skip_logged,
            log=logs.append,
        )
        second_tail_only = archive_policy.resolve_session_mirror_archive_policy(
            "thread-1",
            archive_recommended=True,
            active_output_target=False,
            archive_skip_logged=skip_logged,
            log=logs.append,
        )

        self.assertTrue(first_tail_only)
        self.assertTrue(second_tail_only)
        self.assertEqual(skip_logged, {"thread-1"})
        self.assertEqual(logs, ["session_mirror_archive_tail_only target=thread-1 reason=archive_recommended"])

    def test_active_output_with_prior_skip_logs_override_and_clears_marker(self) -> None:
        skip_logged = {"thread-1"}
        logs: list[str] = []

        archive_tail_only = archive_policy.resolve_session_mirror_archive_policy(
            "thread-1",
            archive_recommended=True,
            active_output_target=True,
            archive_skip_logged=skip_logged,
            log=logs.append,
        )

        self.assertFalse(archive_tail_only)
        self.assertEqual(skip_logged, set())
        self.assertEqual(logs, ["session_mirror_archive_skip_overridden target=thread-1 reason=active_ask"])

    def test_non_archive_recommended_clears_stale_marker_without_log(self) -> None:
        skip_logged = {"thread-1"}
        logs: list[str] = []

        archive_tail_only = archive_policy.resolve_session_mirror_archive_policy(
            "thread-1",
            archive_recommended=False,
            active_output_target=False,
            archive_skip_logged=skip_logged,
            log=logs.append,
        )

        self.assertFalse(archive_tail_only)
        self.assertEqual(skip_logged, set())
        self.assertEqual(logs, [])


class SessionMirrorArchiveCleanupTests(unittest.IsolatedAsyncioTestCase):
    @staticmethod
    def make_deps(
        events: list[str],
        *,
        release_result: bool,
        reactivate_on_release: bool = False,
    ) -> archive_policy.ArchiveMirrorCleanupDeps:
        state = FakeMirrorState()

        async def release(thread_id: str | None) -> bool:
            events.append(f"release:{thread_id}")
            if release_result:
                state.active_output_targets.pop("thread-1", None)
                state.pending_cursor_targets.discard("thread-1")
            if reactivate_on_release:
                state.active_output_targets["thread-1"] = 2.0
            return release_result

        def delete(thread_id: str) -> dict[str, int]:
            events.append(f"delete:{thread_id}")
            return {"mirror_threads": 1, "session_mirror_offsets": 1}

        return archive_policy.ArchiveMirrorCleanupDeps(
            delete_archived_mirror_state=delete,
            get_session_mirror_state=lambda: state,
            normalize_runner_key=lambda value: value or "",
            release_session_mirror_output_target=release,
            parse_bridge_output_value=lambda output, key: "thread-1",
            format_log_argv=lambda argv: " ".join(argv),
            exception_types=(RuntimeError,),
            format_exception=lambda: "traceback",
            log=lambda message: events.append(f"log:{message}"),
        )

    async def test_subscription_is_released_before_archived_state_is_deleted(self) -> None:
        events: list[str] = []

        counts = await archive_policy.cleanup_archived_session_mirror_state(
            None,
            "thread-1",
            deps=self.make_deps(events, release_result=True),
        )

        self.assertEqual(events[:2], ["release:thread-1", "delete:thread-1"])
        self.assertEqual(counts["mirror_threads"], 1)

    async def test_release_failure_preserves_archived_state(self) -> None:
        events: list[str] = []

        warning = await archive_policy.cleanup_archive_mirror_after_bridge_command(
            None,
            ["archive", "--thread-id", "thread-1"],
            0,
            "archived_thread: thread-1",
            deps=self.make_deps(events, release_result=False),
        )

        self.assertNotIn("delete:thread-1", events)
        self.assertIsNotNone(warning)
        self.assertIn("mirror state was preserved", warning or "")

    async def test_reactivation_after_release_preserves_archived_state(self) -> None:
        events: list[str] = []

        warning = await archive_policy.cleanup_archive_mirror_after_bridge_command(
            None,
            ["archive", "--thread-id", "thread-1"],
            0,
            "archived_thread: thread-1",
            deps=self.make_deps(
                events,
                release_result=True,
                reactivate_on_release=True,
            ),
        )

        self.assertNotIn("delete:thread-1", events)
        self.assertIn("reactivated", warning or "")

    def test_active_output_without_prior_skip_does_not_log_override(self) -> None:
        skip_logged: set[str] = set()
        logs: list[str] = []

        archive_tail_only = archive_policy.resolve_session_mirror_archive_policy(
            "thread-1",
            archive_recommended=True,
            active_output_target=True,
            archive_skip_logged=skip_logged,
            log=logs.append,
        )

        self.assertFalse(archive_tail_only)
        self.assertEqual(skip_logged, set())
        self.assertEqual(logs, [])

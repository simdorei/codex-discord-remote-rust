from __future__ import annotations

import unittest

import codex_discord_mirror_access as mirror_access
from codex_discord_mirror_check import (
    MirrorCheckExpectedThread,
    build_mirror_check_expected_threads,
    format_mirror_check_summary,
    summarize_mirror_check,
)
from codex_discord_mirror_rows import MirrorCheckRow
from codex_thread_models import ThreadInfo


def _thread(thread_id: str, title: str, cwd: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=title,
        cwd=cwd,
        updated_at=1,
        rollout_path=f"{thread_id}.jsonl",
        model="gpt-5.5",
        reasoning_effort="xhigh",
        tokens_used=0,
    )


class MirrorCheckHelperTests(unittest.TestCase):
    def test_accessibility_reasons_distinguish_discord_lookup_failures(self) -> None:
        # Given
        class NotFound(RuntimeError):
            pass

        class Forbidden(RuntimeError):
            pass

        # When
        unknown_channel = mirror_access.accessibility_reason_from_fetch_error(
            NotFound("404 Not Found (error code: 10003): Unknown Channel")
        )
        not_found = mirror_access.accessibility_reason_from_fetch_error(NotFound("missing"))
        forbidden = mirror_access.accessibility_reason_from_fetch_error(Forbidden("denied"))
        missing_from_lists = mirror_access.not_in_active_or_archived_thread_lists_reason()

        # Then
        self.assertEqual(unknown_channel, "unknown_channel")
        self.assertEqual(not_found, "not_found")
        self.assertEqual(forbidden, "forbidden")
        self.assertEqual(missing_from_lists, "not_in_active_or_archived_thread_lists")

    def test_formats_clean_summary_when_all_expected_threads_are_mirrored(self) -> None:
        # Given
        threads = [_thread("alpha-00000000", "Alpha", "C:\\repo")]

        # When
        expected = build_mirror_check_expected_threads(
            threads,
            get_project_key_func=lambda thread: thread.cwd,
            get_project_name_func=lambda _thread: "repo",
            get_thread_ui_name_func=lambda _thread_id, _thread: "Alpha UI",
        )
        summary = summarize_mirror_check(
            expected,
            [MirrorCheckRow(codex_thread_id="alpha-00000000", project_key="C:\\repo", discord_thread_id=333)],
        )
        output = format_mirror_check_summary(summary, archive_recommended_count=2)

        # Then
        self.assertEqual(
            expected,
            (
                MirrorCheckExpectedThread(
                    thread_id="alpha-00000000",
                    project_key="C:\\repo",
                    project_name="repo",
                    title="Alpha UI",
                ),
            ),
        )
        self.assertEqual(
            output,
            "\n".join(
                [
                    "Mirror check",
                    "This checks Codex-to-mirror DB mappings only.",
                    "`!mirror sync` removes stale/orphan threads only under Codex mirror project channels.",
                    "General Discord threads are outside this check.",
                    "`rec archive` is only a recommendation; archive first, then sync.",
                    "codex_threads: 1",
                    "mirrored_threads: 1",
                    "missing: 0",
                    "stale: 0",
                    "wrong_project: 0",
                    "archive_recommended: 2",
                ]
            ),
        )

    def test_formats_missing_wrong_project_and_stale_sections(self) -> None:
        # Given
        expected = (
            MirrorCheckExpectedThread(
                thread_id="alpha-00000000",
                project_key="C:\\repo",
                project_name="repo",
                title="Alpha UI",
            ),
            MirrorCheckExpectedThread(
                thread_id="beta-00000000",
                project_key="C:\\repo",
                project_name="repo",
                title="Beta UI",
            ),
        )

        # When
        summary = summarize_mirror_check(
            expected,
            [
                MirrorCheckRow(
                    codex_thread_id="alpha-00000000",
                    project_key="C:\\wrong",
                    discord_thread_id=333,
                ),
                MirrorCheckRow(
                    codex_thread_id="stale-00000000",
                    project_key="C:\\old",
                    discord_thread_id=444,
                ),
            ],
        )
        output = format_mirror_check_summary(summary)

        # Then
        self.assertEqual(
            output,
            "\n".join(
                [
                    "Mirror check",
                    "This checks Codex-to-mirror DB mappings only.",
                    "`!mirror sync` removes stale/orphan threads only under Codex mirror project channels.",
                    "General Discord threads are outside this check.",
                    "`rec archive` is only a recommendation; archive first, then sync.",
                    "codex_threads: 2",
                    "mirrored_threads: 2",
                    "missing: 1",
                    "stale: 1",
                    "wrong_project: 1",
                    "",
                    "Missing:",
                    "- repo / Beta UI (beta-000)",
                    "",
                    "Wrong project:",
                    "- alpha-00 current=C:\\wrong expected=C:\\repo "
                    + "discord_thread_id=333 parent_channel_id=0 accessible=unknown "
                    + "archived=unknown last_seen=0.0 stale=false reason=active_mapping",
                    "",
                    "Stale:",
                    "- stale-00 discord_thread_id=444 parent_channel_id=0 accessible=unknown "
                    + "archived=unknown last_seen=0.0 stale=true "
                    + "reason=not_in_active_or_archived_thread_lists",
                    "",
                    "Run `!mirror sync` to repair.",
                ]
            ),
        )

    def test_formats_repair_hint_when_only_stale_rows_exist(self) -> None:
        # Given
        expected = ()

        # When
        summary = summarize_mirror_check(
            expected,
            [
                MirrorCheckRow(
                    codex_thread_id="stale-00000000",
                    project_key="C:\\old",
                    discord_thread_id=444,
                    parent_channel_id=111,
                    stale=True,
                    reason=mirror_access.UNKNOWN_CHANNEL_REASON,
                ),
            ],
        )
        output = format_mirror_check_summary(summary)

        # Then
        self.assertIn("stale: 1", output)
        self.assertIn("reason=unknown_channel", output)
        self.assertIn("Run `!mirror sync` to repair.", output)


if __name__ == "__main__":
    _ = unittest.main()

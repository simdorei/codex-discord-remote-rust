from __future__ import annotations

import unittest

import codex_discord_logging
import codex_discord_log_summary as log_summary


class DiscordLogSummaryTests(unittest.TestCase):
    def test_summarizes_known_hook_events(self) -> None:
        cases = {
            "[2026-06-18 01:02:03] socket_message_create_untracked channel=234 source=history": (
                "2026-06-18 01:02:03 raw_message_untracked channel=234 source=history"
            ),
            "[2026-06-18 01:02:04] socket_message_create channel=123 source=gateway bot=False content_len=9": (
                "2026-06-18 01:02:04 raw_message channel=123 source=gateway bot=False content_len=9"
            ),
            "[2026-06-18 01:02:05] message_received chat=345 content_len=12": (
                "2026-06-18 01:02:05 message_received channel=345 content_len=12"
            ),
            "[2026-06-18 01:02:06] ignored_message reason=bot chat=456": (
                "2026-06-18 01:02:06 ignored_message reason=bot channel=456"
            ),
            "[2026-06-18 01:02:07] history_poll_message channel=567 content_len=3": (
                "2026-06-18 01:02:07 history_poll_message channel=567 content_len=3"
            ),
            "[2026-06-18 01:02:08] history_poll_primed channel=678 messages=11": (
                "2026-06-18 01:02:08 history_poll_primed channel=678 messages=11"
            ),
            "[2026-06-18 01:02:09] message chat=789 target=thread prefix=! text_len=20": (
                "2026-06-18 01:02:09 message_routed channel=789 target=thread prefix=! text_len=20"
            ),
            "[2026-06-18 01:02:10] socket_interaction_create channel=890 type=2 command=ask": (
                "2026-06-18 01:02:10 raw_interaction channel=890 type=2 command=ask"
            ),
            "[2026-06-18 01:02:11] interaction_received channel=901 type=component command=-": (
                "2026-06-18 01:02:11 interaction_received channel=901 type=component command=-"
            ),
            "[2026-06-18 01:02:12] slash_response_sent channel=902 command=doctor exit=0 response=ok reason=-": (
                "2026-06-18 01:02:12 slash_response_sent channel=902 command=doctor exit=0 response=ok reason=-"
            ),
            "[2026-06-18 01:02:13] component_interaction_unhandled channel=903 custom_id=qa:ok": (
                "2026-06-18 01:02:13 component_event channel=903 custom_id=qa:ok"
            ),
            "[2026-06-18 01:02:14] busy_choice_sent reason=late_busy target=thread-1": (
                "2026-06-18 01:02:14 busy_choice_event reason=late_busy target=thread-1"
            ),
            "[2026-06-18 01:02:15] approval_persistent_done target=thread-2 exit=0": (
                "2026-06-18 01:02:15 approval_persistent target=thread-2 exit=0"
            ),
            "[2026-06-18 01:02:16] input_choice_persistent_done target=thread-3 exit=1": (
                "2026-06-18 01:02:16 input_choice_persistent target=thread-3 exit=1"
            ),
        }

        for line, expected in cases.items():
            with self.subTest(line=line):
                self.assertEqual(log_summary.summarize_discord_hook_log_line(line), expected)

    def test_parses_log_line_and_fields(self) -> None:
        parsed = log_summary.parse_log_line("[2026-06-18 01:02:03] message chat=123 text_len=4")

        self.assertEqual(parsed, ("2026-06-18 01:02:03", "message chat=123 text_len=4"))
        self.assertEqual(log_summary.get_log_field("message chat=123 text_len=4", "chat"), "123")
        self.assertEqual(log_summary.get_log_field("message chat=123 text_len=4", "missing"), "-")

    def test_logging_module_preserves_public_summary_imports(self) -> None:
        line = "[2026-06-18 01:02:03] socket_message_create channel=123 source=gateway bot=False content_len=9"

        self.assertEqual(codex_discord_logging.parse_log_line, log_summary.parse_log_line)
        self.assertEqual(codex_discord_logging.get_log_field, log_summary.get_log_field)
        self.assertEqual(
            codex_discord_logging.summarize_discord_hook_log_line(line),
            log_summary.summarize_discord_hook_log_line(line),
        )
        self.assertEqual(
            codex_discord_logging.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message channel=123 source=gateway bot=False content_len=9"
            ),
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message channel=123 source=gateway bot=False content_len=9"
            ),
        )

    def test_edges_return_none_or_missing_field_markers(self) -> None:
        self.assertIsNone(log_summary.parse_log_line("not a log line"))
        self.assertIsNone(log_summary.summarize_discord_hook_log_line("not a log line"))
        self.assertIsNone(
            log_summary.summarize_discord_hook_log_line("[2026-06-18 01:02:03] unrelated event ignored")
        )
        self.assertEqual(
            log_summary.summarize_discord_hook_log_line("[2026-06-18 01:02:03] socket_message_create channel=123"),
            "2026-06-18 01:02:03 raw_message channel=123 source=- bot=- content_len=-",
        )

    def test_user_or_control_filtering_matches_existing_semantics(self) -> None:
        self.assertTrue(
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message channel=123 source=gateway bot=False content_len=9"
            )
        )
        self.assertTrue(
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message channel=123 source=gateway bot=- content_len=9"
            )
        )
        self.assertFalse(
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message channel=123 source=gateway bot=True content_len=9"
            )
        )
        self.assertTrue(
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 raw_message_untracked channel=123 source=history"
            )
        )
        self.assertTrue(
            log_summary.is_user_or_control_hook_summary(
                "2026-06-18 01:02:03 component_event channel=123 custom_id=qa:ok"
            )
        )
        self.assertFalse(log_summary.is_user_or_control_hook_summary("2026-06-18 01:02:03 ignored"))


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from dataclasses import dataclass, field
import unittest

import codex_discord_prompt_stream_suppression as stream_suppression


@dataclass(frozen=True, slots=True)
class FakeRelay:
    suppressed_after_steering: bool = False
    relay_generation: int = 0
    sent_live: bool = False


@dataclass(slots=True)
class StreamSuppressionFixture:
    stale: bool = False
    stale_calls: list[tuple[str | None, int]] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)

    def is_stale(self, target_thread_id: str | None, relay_generation: int) -> bool:
        self.stale_calls.append((target_thread_id, relay_generation))
        return self.stale

    def build(self) -> stream_suppression.StreamSuppressionDeps:
        return stream_suppression.StreamSuppressionDeps(
            is_discord_relay_stale=self.is_stale,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class StreamSuppressionTests(unittest.TestCase):
    def test_unsuppressed_relay_returns_not_handled_without_log(self) -> None:
        fixture = StreamSuppressionFixture()

        handled = stream_suppression.handle_stream_suppressed_after_steering(
            relay=FakeRelay(suppressed_after_steering=False, relay_generation=3),
            target_thread_id="thread-1",
            output="ordinary",
            deps=fixture.build(),
        )

        self.assertFalse(handled)
        self.assertEqual(fixture.logs, [])
        self.assertEqual(fixture.stale_calls, [])

    def test_stale_relay_logs_newer_relay_suppression(self) -> None:
        fixture = StreamSuppressionFixture(stale=True)

        handled = stream_suppression.handle_stream_suppressed_after_steering(
            relay=FakeRelay(suppressed_after_steering=True, relay_generation=3, sent_live=True),
            target_thread_id="thread-1",
            output="abcdef",
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.stale_calls, [("thread-1", 3)])
        self.assertIn(
            "ask_stream_suppressed_after_newer_relay target=thread-1 sent_live=True output_len=6",
            "\n".join(fixture.logs),
        )

    def test_non_stale_relay_logs_steering_suppression(self) -> None:
        fixture = StreamSuppressionFixture(stale=False)

        handled = stream_suppression.handle_stream_suppressed_after_steering(
            relay=FakeRelay(suppressed_after_steering=True, relay_generation=4, sent_live=False),
            target_thread_id="thread-1",
            output="abc",
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.stale_calls, [("thread-1", 4)])
        self.assertIn(
            "ask_stream_suppressed_after_steering target=thread-1 sent_live=False output_len=3",
            "\n".join(fixture.logs),
        )

    def test_missing_target_logs_dash(self) -> None:
        fixture = StreamSuppressionFixture(stale=False)

        handled = stream_suppression.handle_stream_suppressed_after_steering(
            relay=FakeRelay(suppressed_after_steering=True, relay_generation=5, sent_live=False),
            target_thread_id=None,
            output="abc",
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertIn("target=-", "\n".join(fixture.logs))

import unittest

import codex_thread_context as context
from codex_session_events import JsonEvent


class ThreadContextTests(unittest.TestCase):
    def test_context_usage_detects_peaks_and_inferred_compaction(self) -> None:
        events: list[JsonEvent] = [
            {"type": "event_msg", "payload": {"type": "task_started", "model_context_window": 200000}},
            {
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "model_context_window": 200000,
                        "last_token_usage": {"input_tokens": 90000, "total_tokens": 100000},
                    },
                },
            },
            {
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "model_context_window": 200000,
                        "last_token_usage": {"input_tokens": 30000, "total_tokens": 50000},
                    },
                },
            },
        ]

        usage = context.thread_context_usage_from_events(events)

        self.assertIsNotNone(usage)
        assert usage is not None
        self.assertEqual(usage.model_context_window, 200000)
        self.assertEqual(usage.last_input_tokens, 30000)
        self.assertEqual(usage.peak_input_tokens, 90000)
        self.assertEqual(usage.inferred_compactions, 1)
        self.assertEqual(usage.last_compaction_before_input_tokens, 90000)
        self.assertEqual(usage.last_compaction_after_input_tokens, 30000)
        self.assertEqual(usage.usage_ratio, 0.15)

    def test_context_usage_returns_none_without_token_count(self) -> None:
        self.assertIsNone(
            context.thread_context_usage_from_events(
                [{"type": "event_msg", "payload": {"type": "task_started", "model_context_window": 200000}}]
            )
        )

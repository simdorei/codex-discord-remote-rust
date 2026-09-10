import json
import tempfile
import unittest
from pathlib import Path

import codex_session_events as session_events


class SessionEventsTests(unittest.TestCase):
    def test_iter_session_events_skips_blank_invalid_and_non_object_lines(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text(
                "\n".join(
                    [
                        "",
                        json.dumps({"type": "event_msg", "payload": {"type": "task_started"}}),
                        "{not json",
                        json.dumps(["not", "an", "event"]),
                        json.dumps({"type": "turn_context", "payload": {"service_tier": "priority"}}),
                    ]
                )
                + "\n",
                encoding="utf-8",
            )

            self.assertEqual(
                list(session_events.iter_session_events(session_path)),
                [
                    {"type": "event_msg", "payload": {"type": "task_started"}},
                    {"type": "turn_context", "payload": {"service_tier": "priority"}},
                ],
            )

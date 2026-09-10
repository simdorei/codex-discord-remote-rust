from __future__ import annotations

import json
import tempfile
import unittest
from collections.abc import Iterator
from datetime import datetime, timezone
from pathlib import Path

from codex_discord_weekly_usage import (
    build_weekly_usage_message,
    scan_weekly_usage_events,
)
from codex_session_events import (
    JsonEvent,
    JsonValue,
    iter_session_events as iter_json_session_events,
)


class FakeWeeklyUsageBridge:
    def __init__(
        self,
        codex_home: Path,
        *,
        broken_files: set[str] | None = None,
        crash_files: set[str] | None = None,
    ) -> None:
        self.CODEX_HOME: Path = codex_home
        self.broken_files: set[str] = broken_files or set()
        self.crash_files: set[str] = crash_files or set()

    def coerce_nonnegative_int(self, value: JsonValue | None) -> int:
        if not isinstance(value, (int, float, str)):
            return 0
        try:
            number = int(value)
        except (TypeError, ValueError):
            return 0
        return max(0, number)

    def format_timestamp(self, unix_seconds: int) -> str:
        return datetime.fromtimestamp(unix_seconds, timezone.utc).strftime("%Y-%m-%d")

    def format_token_k(self, value: int) -> str:
        return f"{value / 1000:.1f}k"

    def iter_session_events(self, session_path: Path) -> Iterator[JsonEvent]:
        if session_path.name in self.broken_files:
            raise OSError("bad session file")
        if session_path.name in self.crash_files:
            raise RuntimeError("unexpected bridge bug")
        yield from iter_json_session_events(session_path)


def write_jsonl(path: Path, *events: JsonEvent) -> None:
    _ = path.write_text("".join(json.dumps(event) + "\n" for event in events), encoding="utf-8")


class WeeklyUsageScanTests(unittest.TestCase):
    def test_scan_weekly_usage_events_collects_tokens_threads_and_latest_rate_limits(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            codex_home = Path(temp_dir)
            sessions_dir = codex_home / "sessions"
            sessions_dir.mkdir()
            session_path = sessions_dir / "thread.jsonl"
            first = "2026-06-18T01:00:00Z"
            latest = "2026-06-18T02:00:00Z"
            write_jsonl(
                session_path,
                {"timestamp": first, "type": "session_meta", "payload": {"id": "thread-1"}},
                {"timestamp": first, "type": "event_msg", "payload": {"type": "task_started", "turn_id": "turn-1"}},
                {
                    "timestamp": first,
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "rate_limits": {"limit_id": "old", "primary": {"used_percent": 10}},
                        "info": {"last_token_usage": {"input_tokens": "100", "total_tokens": "150"}},
                    },
                },
                {
                    "timestamp": latest,
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "rate_limits": {"limit_id": "latest", "plan_type": "pro"},
                        "info": {"last_token_usage": {"input_tokens": 20, "total_tokens": 70}},
                    },
                },
            )
            bridge = FakeWeeklyUsageBridge(codex_home)

            result = scan_weekly_usage_events(
                sessions_dir,
                datetime(2026, 6, 17, tzinfo=timezone.utc),
                bridge_module=bridge,
            )

        self.assertEqual(result.turns, 1)
        self.assertEqual(result.token_events, 2)
        self.assertEqual(result.total_tokens, 220)
        self.assertEqual(result.input_tokens, 120)
        self.assertEqual(result.output_tokens, 100)
        self.assertEqual(result.files_scanned, 1)
        self.assertEqual(result.recent_threads, frozenset({"thread-1", "turn-1"}))
        self.assertEqual(result.latest_rate_limits, {"limit_id": "latest", "plan_type": "pro"})
        self.assertEqual(result.latest_rate_limits_at, datetime(2026, 6, 18, 2, 0, tzinfo=timezone.utc))

    def test_scan_weekly_usage_events_ignores_malformed_old_and_bad_files(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            codex_home = Path(temp_dir)
            sessions_dir = codex_home / "sessions"
            sessions_dir.mkdir()
            good_path = sessions_dir / "good.jsonl"
            bad_path = sessions_dir / "bad.jsonl"
            write_jsonl(
                good_path,
                {"timestamp": "2026-06-01T00:00:00Z", "type": "event_msg", "payload": {"type": "task_started"}},
                {"timestamp": "not-a-time", "type": "event_msg", "payload": {"type": "task_started"}},
                {"timestamp": "2026-06-18T01:00:00Z", "type": "event_msg", "payload": []},
                {
                    "timestamp": "2026-06-18T01:00:00Z",
                    "type": "event_msg",
                    "payload": {"type": "token_count", "info": ["not", "a", "dict"]},
                },
                {
                    "timestamp": "2026-06-18T01:00:00Z",
                    "type": "event_msg",
                    "payload": {"type": "token_count", "info": {"last_token_usage": ["not", "a", "dict"]}},
                },
            )
            _ = bad_path.write_text("ignored\n", encoding="utf-8")
            bridge = FakeWeeklyUsageBridge(codex_home, broken_files={"bad.jsonl"})

            result = scan_weekly_usage_events(
                sessions_dir,
                datetime(2026, 6, 17, tzinfo=timezone.utc),
                bridge_module=bridge,
            )

        self.assertEqual(result.turns, 0)
        self.assertEqual(result.token_events, 0)
        self.assertEqual(result.total_tokens, 0)
        self.assertEqual(result.input_tokens, 0)
        self.assertEqual(result.output_tokens, 0)
        self.assertEqual(result.files_scanned, 2)
        self.assertEqual(result.recent_threads, frozenset())
        self.assertIsNone(result.latest_rate_limits)
        self.assertIsNone(result.latest_rate_limits_at)

    def test_scan_weekly_usage_events_surfaces_unexpected_bridge_errors(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            codex_home = Path(temp_dir)
            sessions_dir = codex_home / "sessions"
            sessions_dir.mkdir()
            crash_path = sessions_dir / "crash.jsonl"
            _ = crash_path.write_text("", encoding="utf-8")
            bridge = FakeWeeklyUsageBridge(codex_home, crash_files={"crash.jsonl"})

            with self.assertRaisesRegex(RuntimeError, "unexpected bridge bug"):
                _ = scan_weekly_usage_events(
                    sessions_dir,
                    datetime(2026, 6, 17, tzinfo=timezone.utc),
                    bridge_module=bridge,
                )

if __name__ == "__main__":
    _ = unittest.main()

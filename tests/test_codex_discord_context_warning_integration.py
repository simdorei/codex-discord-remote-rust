from __future__ import annotations

from pathlib import Path
import json
import tempfile
import unittest

from codex_desktop_bridge_formatting import format_token_k
import codex_desktop_bridge_thread_context as thread_context
import codex_discord_context_status as context_status
from codex_thread_models import ThreadContextUsage, ThreadInfo


JsonEvent = dict[str, object]


class ContextThreadMissing(Exception):
    pass


class ContextBridgeFake:
    def __init__(self, thread: ThreadInfo) -> None:
        self.thread: ThreadInfo = thread

    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsage | None:
        return thread_context.get_thread_context_usage(thread)

    def describe_thread_context_usage(self, context_usage: ThreadContextUsage) -> str:
        return thread_context.describe_thread_context_usage(context_usage)

    def should_recommend_archive(
        self,
        thread: ThreadInfo,
        context_usage: ThreadContextUsage | None,
    ) -> bool:
        return thread_context.should_recommend_archive(thread, context_usage)

    def format_token_k(self, value: int) -> str:
        return format_token_k(value)

    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        _ = cwd
        if thread_id != self.thread.id:
            raise ContextThreadMissing(thread_id or "-")
        return self.thread

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str:
        return thread.id

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None:
        _ = thread_id
        return (thread or self.thread).title

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]:
        return [self.thread][:limit]


def task_started_event(context_window: int) -> JsonEvent:
    return {
        "type": "event_msg",
        "payload": {
            "type": "task_started",
            "model_context_window": context_window,
        },
    }


def token_count_event(context_window: int, input_tokens: int, total_tokens: int) -> JsonEvent:
    return {
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "model_context_window": context_window,
                "last_token_usage": {
                    "input_tokens": input_tokens,
                    "total_tokens": total_tokens,
                },
            },
        },
    }


def write_context_thread(
    temp_dir: str,
    events: list[JsonEvent],
    *,
    tokens_used: int = 1,
) -> ThreadInfo:
    session_path = Path(temp_dir) / "session.jsonl"
    _ = session_path.write_text(
        "\n".join(json.dumps(event) for event in events),
        encoding="utf-8",
    )
    return ThreadInfo(
        id="thread-1",
        title="title",
        cwd=str(Path(temp_dir)),
        updated_at=1,
        rollout_path=str(session_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=tokens_used,
    )


def resolve_thread_1(target_thread_id: str | None) -> tuple[str | None, str]:
    _ = target_thread_id
    return "thread-1", "taxlab:1"


class DiscordContextWarningIntegrationTests(unittest.TestCase):
    def test_context_usage_detects_inferred_compaction_drop(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            thread = write_context_thread(
                temp_dir,
                [
                    task_started_event(200000),
                    token_count_event(200000, 100000, 101000),
                    token_count_event(200000, 0, 13000),
                    token_count_event(200000, 35000, 36000),
                ],
            )
            usage = thread_context.get_thread_context_usage(thread)
            context_line = context_status.format_context_usage_line(
                thread,
                bridge_module=ContextBridgeFake(thread),
            )

        if usage is None:
            self.fail("expected context usage")
        self.assertEqual(usage.inferred_compactions, 1)
        self.assertEqual(usage.last_compaction_before_input_tokens, 100000)
        self.assertEqual(usage.last_compaction_after_input_tokens, 35000)
        self.assertIn("compactions=1", context_line)

    def test_context_warning_stays_quiet_below_high_threshold(self) -> None:
        logs: list[str] = []
        with tempfile.TemporaryDirectory() as temp_dir:
            thread = write_context_thread(
                temp_dir,
                [
                    task_started_event(300000),
                    token_count_event(300000, 240000, 240500),
                    token_count_event(300000, 0, 13000),
                    token_count_event(300000, 40000, 40500),
                ],
            )
            bridge_module = ContextBridgeFake(thread)

            warning = context_status.build_context_warning(
                "thread-1",
                bridge_module=bridge_module,
                resolve_target_ref_func=resolve_thread_1,
                log_func=logs.append,
            )
            context_line = context_status.format_context_usage_line(
                thread,
                bridge_module=bridge_module,
            )

        self.assertEqual(warning, "")
        self.assertEqual(logs, [])
        self.assertIn("compactions=1", context_line)
        self.assertIn("archive_recommended=yes", context_line)

    def test_context_status_marks_no_visible_reply_state(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            thread = write_context_thread(
                temp_dir,
                [
                    task_started_event(258400),
                    token_count_event(258400, 236419, 236573),
                    token_count_event(258400, 0, 0),
                ],
                tokens_used=258400,
            )

            context_line = context_status.format_context_usage_line(
                thread,
                bridge_module=ContextBridgeFake(thread),
            )

        self.assertIn("no-visible-reply", context_line)
        self.assertIn("peak=91.5%", context_line)
        self.assertNotIn("(normal)", context_line)

    def test_context_warning_starts_at_high_threshold(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            thread = write_context_thread(
                temp_dir,
                [
                    task_started_event(200000),
                    token_count_event(200000, 140000, 140500),
                ],
                tokens_used=175_000_000,
            )

            warning = context_status.build_context_warning(
                "thread-1",
                bridge_module=ContextBridgeFake(thread),
                resolve_target_ref_func=resolve_thread_1,
                log_func=lambda message: None,
            )

        self.assertIn("Context warning: 70.0% (high)", warning)
        self.assertIn("token_used_total=175.0M", warning)


if __name__ == "__main__":
    _ = unittest.main()

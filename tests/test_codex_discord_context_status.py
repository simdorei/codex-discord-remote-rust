from __future__ import annotations

import unittest

from codex_discord_context_status import build_context_message, build_context_warning
from codex_thread_models import ThreadContextUsage, ThreadInfo


class TargetResolutionError(RuntimeError):
    pass


class ThreadLookupError(RuntimeError):
    pass


class FakeContextBridge:
    def __init__(
        self,
        *,
        choose_error: RuntimeError | None = None,
        context_usage: ThreadContextUsage | None = None,
    ) -> None:
        self.choose_error: RuntimeError | None = choose_error
        self.context_usage: ThreadContextUsage | None = context_usage
        self.thread: ThreadInfo = ThreadInfo(
            id="thread-1",
            title="Root",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="thread-1.jsonl",
            model="gpt-5.5",
            reasoning_effort="xhigh",
            tokens_used=1200,
        )

    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsage | None:
        _ = thread
        return self.context_usage

    def describe_thread_context_usage(self, context_usage: ThreadContextUsage) -> str:
        _ = context_usage
        return "high"

    def should_recommend_archive(
        self,
        thread: ThreadInfo,
        context_usage: ThreadContextUsage | None,
    ) -> bool:
        _ = thread, context_usage
        return False

    def format_token_k(self, value: int) -> str:
        return f"{value / 1000:.1f}k"

    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        _ = thread_id, cwd
        if self.choose_error is not None:
            raise self.choose_error
        return self.thread

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str:
        return thread.id

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None:
        _ = thread_id
        return (thread or self.thread).title

    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]:
        return [self.thread][:limit]


class ContextStatusTests(unittest.TestCase):
    def test_context_warning_logs_and_suppresses_target_resolution_failure(self) -> None:
        logs: list[str] = []

        def raise_target_resolution(_target_thread_id: str | None) -> tuple[str | None, str]:
            raise TargetResolutionError("target missing")

        warning = build_context_warning(
            "thread-1",
            bridge_module=FakeContextBridge(),
            resolve_target_ref_func=raise_target_resolution,
            log_func=logs.append,
        )

        self.assertEqual(warning, "")
        self.assertEqual(
            logs,
            ["context_warning_unavailable target=thread-1 error=target missing"],
        )

    def test_context_message_reports_thread_lookup_failure(self) -> None:
        message = build_context_message(
            123,
            bridge_module=FakeContextBridge(choose_error=ThreadLookupError("unknown thread")),
            get_mirrored_codex_thread_id_func=lambda channel_id: "thread-1",
            resolve_selected_target_func=lambda: (None, "-"),
        )

        self.assertEqual(message, "Context unavailable.\n\nERROR: unknown thread")


if __name__ == "__main__":
    _ = unittest.main()

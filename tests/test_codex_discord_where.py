from __future__ import annotations

from typing import override
import unittest

import codex_discord_where as where
from codex_thread_models import ThreadInfo


class MissingThreadError(RuntimeError):
    pass


class UnexpectedWhereBridgeError(Exception):
    pass


class FakeBridge:
    def __init__(self) -> None:
        self.thread: ThreadInfo = ThreadInfo(
            id="thread-1",
            title="Title",
            cwd="C:/repo",
            updated_at=1,
            rollout_path="session.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=12345,
        )

    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        _ = cwd
        if thread_id != "thread-1":
            raise MissingThreadError("missing")
        return self.thread

    def get_thread_busy_state(self, thread: ThreadInfo, *, allow_resume: bool = False) -> str | None:
        _ = thread
        _ = allow_resume
        return None

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str:
        _ = thread
        return "repo:1"

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None:
        _ = thread_id
        _ = thread
        return "UI Title"

    def format_token_k(self, value: int) -> str:
        _ = value
        return "12.3k"


class BrokenBridge(FakeBridge):
    @override
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        _ = (thread_id, cwd)
        raise UnexpectedWhereBridgeError("boom")


class DiscordWhereTests(unittest.TestCase):
    def test_build_where_message_for_mapped_thread(self) -> None:
        output = where.build_where_message(
            222,
            bridge_module=FakeBridge(),
            get_mirrored_codex_thread_id_func=lambda channel_id: "thread-1",
            describe_mirrored_project_channel_func=lambda channel_id: "",
            format_context_usage_line_func=lambda thread: "context: ok",
        )

        self.assertIn("Mapped Codex thread", output)
        self.assertIn("thread_ref: repo:1", output)
        self.assertIn("title: UI Title", output)
        self.assertIn("state: idle", output)
        self.assertIn("context: ok", output)

    def test_build_where_message_for_unmapped_channel(self) -> None:
        output = where.build_where_message(
            222,
            bridge_module=FakeBridge(),
            get_mirrored_codex_thread_id_func=lambda channel_id: None,
            describe_mirrored_project_channel_func=lambda channel_id: "",
            format_context_usage_line_func=lambda thread: "context: ok",
        )

        self.assertEqual(output, "This Discord channel is not mapped to a Codex thread.")

    def test_build_where_message_for_missing_mapped_thread_returns_error(self) -> None:
        output = where.build_where_message(
            222,
            bridge_module=FakeBridge(),
            get_mirrored_codex_thread_id_func=lambda channel_id: "missing-thread",
            describe_mirrored_project_channel_func=lambda channel_id: "",
            format_context_usage_line_func=lambda thread: "context: ok",
        )

        self.assertEqual(output, "Mapped Codex thread: missing-thread\nERROR: missing")

    def test_build_where_message_surfaces_unexpected_bridge_failure(self) -> None:
        with self.assertRaisesRegex(UnexpectedWhereBridgeError, "boom"):
            _ = where.build_where_message(
                222,
                bridge_module=BrokenBridge(),
                get_mirrored_codex_thread_id_func=lambda channel_id: "thread-1",
                describe_mirrored_project_channel_func=lambda channel_id: "",
                format_context_usage_line_func=lambda thread: "context: ok",
            )


if __name__ == "__main__":
    _ = unittest.main()

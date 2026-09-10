from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import codex_desktop_bridge_thread_list as thread_list
from codex_bridge_state import JsonObject
from codex_thread_models import ThreadContextUsage, ThreadInfo


class SidecarUnavailableError(RuntimeError):
    pass


class UnexpectedSidecarError(Exception):
    pass


class FakeSidecar:
    def __init__(self) -> None:
        self.closed: bool = False

    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject:
        _ = (thread_id, include_turns)
        return {}

    def close(self) -> None:
        self.closed = True


def _thread(rollout_path: Path) -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread title",
        cwd="C:/repo",
        updated_at=1,
        rollout_path=str(rollout_path),
        model="gpt-5.5",
        reasoning_effort="high",
        tokens_used=42,
    )


def _deps(
    lines: list[str],
    new_sidecar: thread_list.NewSidecar,
) -> thread_list.ThreadListDeps:
    def get_thread_context_usage(thread: ThreadInfo) -> ThreadContextUsage | None:
        _ = thread
        return None

    return thread_list.ThreadListDeps(
        get_selected_thread_id=lambda: None,
        build_workspace_ref_map=lambda threads: {thread.id: "repo" for thread in threads},
        get_thread_ui_name=lambda thread_id, thread: None,
        collapse_list_text=lambda text, limit: text[:limit],
        get_thread_workspace_name=lambda thread: Path(thread.cwd).name,
        is_thread_busy=lambda path: path.exists(),
        new_sidecar=new_sidecar,
        get_thread_busy_state=lambda thread, sidecar, include_turns: "sidecar-busy" if sidecar else "busy",
        get_thread_context_usage=get_thread_context_usage,
        format_token_k=lambda tokens: str(tokens),
        should_recommend_archive=lambda thread, usage: False,
        format_thread_model_display=lambda thread, collaboration, service_tier: (
            f"{thread.model}/{thread.reasoning_effort}/{collaboration}/{service_tier}"
        ),
        get_thread_collaboration_mode=lambda thread: "default",
        get_thread_service_tier=lambda thread: "fast",
        format_timestamp=lambda timestamp: f"ts:{timestamp}",
        make_console_safe_text=lambda text: text,
        get_live_pending_approval_display_lines=lambda thread, timeout: (None, []),
        summarize_interactive_lines=lambda state, lines: "",
        get_pending_interactive_summary=lambda path: "",
        print_line=lines.append,
    )


class DesktopBridgeThreadListTests(unittest.TestCase):
    def test_print_thread_list_preserves_expected_sidecar_failure_fallback(self) -> None:
        lines: list[str] = []

        def new_sidecar() -> thread_list.SidecarClient:
            raise SidecarUnavailableError("sidecar unavailable")

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("{}", encoding="utf-8")

            thread_list.print_thread_list([_thread(session_path)], _deps(lines, new_sidecar))

        self.assertEqual(len(lines), 1)
        self.assertIn("| busy", lines[0])
        self.assertNotIn("sidecar-busy", lines[0])

    def test_print_thread_list_surfaces_unexpected_sidecar_failure(self) -> None:
        lines: list[str] = []

        def new_sidecar() -> thread_list.SidecarClient:
            raise UnexpectedSidecarError("sidecar dependency broke")

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("{}", encoding="utf-8")

            with self.assertRaisesRegex(UnexpectedSidecarError, "sidecar dependency broke"):
                thread_list.print_thread_list([_thread(session_path)], _deps(lines, new_sidecar))

        self.assertEqual(lines, [])


if __name__ == "__main__":
    _ = unittest.main()

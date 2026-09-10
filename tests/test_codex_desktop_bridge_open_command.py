from __future__ import annotations

import unittest
from pathlib import Path

import codex_desktop_bridge_open_command as open_command
from codex_thread_models import ThreadInfo


class ActivationMissingError(RuntimeError):
    pass


class UnexpectedActivationError(Exception):
    pass


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread title",
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt-5.5",
        reasoning_effort="high",
        tokens_used=42,
    )


def _deps(
    lines: list[str],
    selected_ids: list[str],
    activate_thread_in_ui: open_command.ActivateThreadInUi,
) -> open_command.OpenCommandDeps:
    def get_busy_threads(limit: int) -> list[ThreadInfo]:
        _ = limit
        return []

    def get_thread_label(thread: ThreadInfo) -> str:
        return thread.id

    def cancel_codex_reply_if_busy(timeout: float) -> tuple[list[str], list[str]]:
        _ = timeout
        raise AssertionError("cancel should not run")

    def get_last_messages(rollout_path: Path) -> tuple[str, str]:
        return f"user:{rollout_path.name}", "assistant:last"

    def format_title_preview(title: str) -> str:
        return f"preview:{title}"

    def get_thread_ui_name(thread_id: str, thread: ThreadInfo) -> str | None:
        _ = thread
        return f"ui:{thread_id}"

    return open_command.OpenCommandDeps(
        get_busy_threads=get_busy_threads,
        get_thread_label=get_thread_label,
        cancel_codex_reply_if_busy=cancel_codex_reply_if_busy,
        set_selected_thread_id=selected_ids.append,
        activate_thread_in_ui=activate_thread_in_ui,
        get_last_user_and_assistant_messages=get_last_messages,
        format_title_preview=format_title_preview,
        get_thread_ui_name=get_thread_ui_name,
        print_line=lines.append,
    )


class DesktopBridgeOpenCommandTests(unittest.TestCase):
    def test_run_open_command_renders_expected_activation_failure(self) -> None:
        lines: list[str] = []
        selected_ids: list[str] = []

        def activate_thread_in_ui(thread: ThreadInfo) -> str:
            _ = thread
            raise ActivationMissingError("window unavailable")

        open_command.run_open_command(
            _thread(),
            abort=False,
            deps=_deps(lines, selected_ids, activate_thread_in_ui),
        )

        self.assertEqual(selected_ids, ["thread-1"])
        self.assertIn("ui_activation: best-effort (unverified)", lines)
        self.assertIn("ui_warning: window unavailable", lines)
        self.assertIn("[last_user]", lines)
        self.assertIn("user:session.jsonl", lines)

    def test_run_open_command_surfaces_unexpected_activation_failure(self) -> None:
        lines: list[str] = []
        selected_ids: list[str] = []

        def activate_thread_in_ui(thread: ThreadInfo) -> str:
            _ = thread
            raise UnexpectedActivationError("dependency broke")

        with self.assertRaisesRegex(UnexpectedActivationError, "dependency broke"):
            open_command.run_open_command(
                _thread(),
                abort=False,
                deps=_deps(lines, selected_ids, activate_thread_in_ui),
            )

        self.assertEqual(selected_ids, ["thread-1"])
        self.assertEqual(lines, [])


if __name__ == "__main__":
    _ = unittest.main()

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from codex_thread_models import ThreadInfo


FormatTitlePreview = Callable[[str], str]
GetInteractiveDisplayLines = Callable[[Path], tuple[str | None, list[str]]]
GetLastMessages = Callable[[Path], tuple[str, str]]
GetLivePendingApprovalDisplayLines = Callable[[ThreadInfo, float], tuple[str | None, list[str]]]
GetThreadBusyState = Callable[[ThreadInfo], str]
GetThreadUiName = Callable[[str, ThreadInfo], str | None]
PrintLine = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class UseReportDeps:
    get_last_user_and_assistant_messages: GetLastMessages
    get_thread_busy_state: GetThreadBusyState
    get_live_pending_approval_display_lines: GetLivePendingApprovalDisplayLines
    get_pending_interactive_display_lines: GetInteractiveDisplayLines
    format_title_preview: FormatTitlePreview
    get_thread_ui_name: GetThreadUiName
    print_line: PrintLine


def print_use_report(thread: ThreadInfo, deps: UseReportDeps) -> None:
    session_path = Path(thread.rollout_path)
    last_user, last_assistant = deps.get_last_user_and_assistant_messages(session_path)
    interactive_state, interactive_lines = _interactive_lines(thread, session_path, deps)
    deps.print_line(f"selected_thread: {thread.id}")
    _print_labeled_block("last_user", last_user, deps.print_line)
    _print_labeled_block("last_assistant", last_assistant, deps.print_line)
    if interactive_lines:
        deps.print_line("")
        deps.print_line(f"[{interactive_state}]")
        for line in interactive_lines:
            deps.print_line(line)
    deps.print_line("")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"ui_name: {deps.get_thread_ui_name(thread.id, thread) or '-'}")
    deps.print_line(f"cwd: {thread.cwd}")


def _interactive_lines(
    thread: ThreadInfo,
    session_path: Path,
    deps: UseReportDeps,
) -> tuple[str | None, list[str]]:
    interactive_state: str | None = None
    interactive_lines: list[str] = []
    if deps.get_thread_busy_state(thread) == "waiting-approval":
        interactive_state, interactive_lines = deps.get_live_pending_approval_display_lines(thread, 1.0)
    if interactive_lines:
        return interactive_state, interactive_lines
    return deps.get_pending_interactive_display_lines(session_path)


def _print_labeled_block(label: str, text: str, print_line: PrintLine) -> None:
    if not text:
        return
    print_line("")
    print_line(f"[{label}]")
    print_line(text)

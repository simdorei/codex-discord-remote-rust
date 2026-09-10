from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from codex_thread_models import ThreadContextUsage, ThreadInfo


DescribeThreadContextUsage = Callable[[ThreadContextUsage], str]
FormatTimestamp = Callable[[int], str]
GetLastMessages = Callable[[Path], tuple[str, str]]
GetThreadBusyState = Callable[[ThreadInfo], str]
GetThreadContextUsage = Callable[[ThreadInfo], ThreadContextUsage | None]
GetThreadSlot = Callable[[ThreadInfo], int | None]
GetThreadText = Callable[[ThreadInfo], str]
GetThreadUiName = Callable[[str, ThreadInfo], str | None]
PrintLine = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class StatusReportDeps:
    get_last_user_and_assistant_messages: GetLastMessages
    get_thread_busy_state: GetThreadBusyState
    get_thread_slot: GetThreadSlot
    get_thread_ui_name: GetThreadUiName
    get_thread_context_usage: GetThreadContextUsage
    get_thread_workspace_ref: GetThreadText
    format_timestamp: FormatTimestamp
    describe_thread_context_usage: DescribeThreadContextUsage
    print_line: PrintLine


def print_thread_status(thread: ThreadInfo, deps: StatusReportDeps) -> None:
    session_path = Path(thread.rollout_path)
    last_user, last_assistant = deps.get_last_user_and_assistant_messages(session_path)
    busy_state = deps.get_thread_busy_state(thread)
    busy = busy_state != "idle"
    slot = deps.get_thread_slot(thread)
    ui_name = deps.get_thread_ui_name(thread.id, thread)
    context_usage = deps.get_thread_context_usage(thread)
    deps.print_line(f"thread_id: {thread.id}")
    deps.print_line(f"thread_ref: {deps.get_thread_workspace_ref(thread)}")
    deps.print_line(f"title: {thread.title}")
    deps.print_line(f"ui_name: {ui_name or '-'}")
    deps.print_line(f"cwd: {thread.cwd}")
    deps.print_line(f"updated_at: {deps.format_timestamp(thread.updated_at)}")
    deps.print_line(f"model: {thread.model} / {thread.reasoning_effort}")
    deps.print_line(f"tokens_used: {thread.tokens_used}")
    _print_context_usage(context_usage, deps)
    deps.print_line(f"ui_slot: {slot if slot is not None else '-'}")
    deps.print_line(f"busy: {busy}")
    deps.print_line(f"busy_state: {busy_state}")
    deps.print_line(f"session_path: {session_path}")
    deps.print_line("")
    _print_last_block("last_user", last_user, deps.print_line)
    _print_last_block("last_assistant", last_assistant, deps.print_line)


def _print_context_usage(
    context_usage: ThreadContextUsage | None,
    deps: StatusReportDeps,
) -> None:
    if context_usage is None:
        deps.print_line("context_usage: -")
        return
    deps.print_line(f"context_window: {context_usage.model_context_window}")
    deps.print_line(f"last_input_tokens: {context_usage.last_input_tokens}")
    deps.print_line(f"last_total_tokens: {context_usage.last_total_tokens}")
    usage_text = f"{context_usage.usage_ratio * 100:.1f}% ({deps.describe_thread_context_usage(context_usage)})"
    deps.print_line(f"context_usage: {usage_text}")


def _print_last_block(label: str, text: str, print_line: PrintLine) -> None:
    if not text:
        return
    print_line(f"[{label}]")
    print_line(text)
    if label == "last_user":
        print_line("")

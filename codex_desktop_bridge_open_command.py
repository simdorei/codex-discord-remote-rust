from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from codex_thread_models import ThreadInfo


ActivateThreadInUi = Callable[[ThreadInfo], str]
CancelCodexReplyIfBusy = Callable[[float], tuple[list[str], list[str]]]
FormatTitlePreview = Callable[[str], str]
GetBusyThreads = Callable[[int], list[ThreadInfo]]
GetLastMessages = Callable[[Path], tuple[str, str]]
GetThreadLabel = Callable[[ThreadInfo], str]
GetThreadUiName = Callable[[str, ThreadInfo], str | None]
PrintLine = Callable[[str], None]
SetSelectedThreadId = Callable[[str], None]


@dataclass(frozen=True, slots=True)
class OpenCommandDeps:
    get_busy_threads: GetBusyThreads
    get_thread_label: GetThreadLabel
    cancel_codex_reply_if_busy: CancelCodexReplyIfBusy
    set_selected_thread_id: SetSelectedThreadId
    activate_thread_in_ui: ActivateThreadInUi
    get_last_user_and_assistant_messages: GetLastMessages
    format_title_preview: FormatTitlePreview
    get_thread_ui_name: GetThreadUiName
    print_line: PrintLine


def run_open_command(thread: ThreadInfo, *, abort: bool, deps: OpenCommandDeps) -> None:
    cancelled_labels, cancel_remaining = _cancel_other_busy_threads(thread, abort, deps)
    deps.set_selected_thread_id(thread.id)
    activation_method, activation_warning = _activate_thread(thread, deps.activate_thread_in_ui)
    last_user, last_assistant = deps.get_last_user_and_assistant_messages(Path(thread.rollout_path))

    deps.print_line(f"selected_thread: {thread.id}")
    deps.print_line(f"target_thread: {thread.id}")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"ui_name: {deps.get_thread_ui_name(thread.id, thread) or '-'}")
    deps.print_line(f"cwd: {thread.cwd}")
    _print_cancel_status(cancelled_labels, cancel_remaining, deps.print_line)
    deps.print_line(f"ui_activation: {activation_method}")
    if activation_warning:
        deps.print_line(f"ui_warning: {activation_warning}")
    _print_labeled_block("last_user", last_user, deps.print_line)
    _print_labeled_block("last_assistant", last_assistant, deps.print_line)


def _cancel_other_busy_threads(
    thread: ThreadInfo,
    abort: bool,
    deps: OpenCommandDeps,
) -> tuple[list[str], list[str]]:
    busy_now = deps.get_busy_threads(50)
    busy_ids = {item.id for item in busy_now}
    if not busy_now or thread.id in busy_ids:
        return [], []
    labels = ", ".join(deps.get_thread_label(item) for item in busy_now[:3])
    if not abort:
        message = (
            "The current thread has a reply in progress. Changing threads will stop it. "
            f"Active thread(s): {labels}. Wait for `[ready]` or rerun with `open --abort ...`."
        )
        raise RuntimeError(message)
    return deps.cancel_codex_reply_if_busy(3.0)


def _activate_thread(
    thread: ThreadInfo,
    activate_thread_in_ui: ActivateThreadInUi,
) -> tuple[str, str]:
    try:
        return activate_thread_in_ui(thread), ""
    except RuntimeError as exc:
        return "best-effort (unverified)", str(exc)


def _print_cancel_status(
    cancelled_labels: list[str],
    cancel_remaining: list[str],
    print_line: PrintLine,
) -> None:
    if not cancelled_labels:
        return
    print_line(f"reply_abort_requested: {', '.join(cancelled_labels)}")
    if cancel_remaining:
        print_line(f"reply_abort_pending: {', '.join(cancel_remaining)}")


def _print_labeled_block(label: str, text: str, print_line: PrintLine) -> None:
    if not text:
        return
    print_line("")
    print_line(f"[{label}]")
    print_line(text)

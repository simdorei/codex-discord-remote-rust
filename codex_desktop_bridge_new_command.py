from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn, Protocol

from codex_thread_models import ThreadInfo


class BackgroundRunner(Protocol):
    @property
    def pid(self) -> int: ...

    def poll(self) -> int | None: ...


CancelCodexReplyIfBusy = Callable[[float], tuple[list[str], list[str]]]
FormatTitlePreview = Callable[[str], str]
GetThreadLabel = Callable[[ThreadInfo], str]
GetThreadUiName = Callable[[str, ThreadInfo], str | None]
LoadRecentThreads = Callable[[int], list[ThreadInfo]]
PrintLine = Callable[[str], None]
ResolveNewThreadCwd = Callable[[str | None], str]
SetSelectedThreadId = Callable[[str], None]
SpawnBackgroundNewThreadRunner = Callable[[str, str], BackgroundRunner]
SyncSessionIndex = Callable[[], int | None]
WaitForNewThread = Callable[[set[str], float], ThreadInfo | None]
WaitForPromptDelivery = Callable[[dict[str, tuple[ThreadInfo, Path, int]], str, float], ThreadInfo | None]

NEW_COMMAND_MISSING_PROMPT_MESSAGE = (
    "Background `new` requires an initial prompt. "
    "The local runner only persists a new thread once the first message is sent."
)
NEW_COMMAND_THREAD_TIMEOUT_MESSAGE = (
    "Background new-thread runner started, but a new persisted thread did not appear in local Codex state in time."
)
NEW_COMMAND_PROMPT_DELIVERY_MESSAGE = "Prompt delivery could not be confirmed in the newly created thread."


class NewCommandMissingPromptError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(NEW_COMMAND_MISSING_PROMPT_MESSAGE)


class NewCommandThreadTimeoutError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(NEW_COMMAND_THREAD_TIMEOUT_MESSAGE)


class NewCommandRunnerExitedError(RuntimeError):
    def __init__(self, exit_code: int) -> None:
        self.exit_code: int = exit_code
        super().__init__(
            f"Background new-thread runner exited before a new thread appeared in local Codex state (exit={exit_code})."
        )


class NewCommandPromptDeliveryError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(NEW_COMMAND_PROMPT_DELIVERY_MESSAGE)


@dataclass(frozen=True, slots=True)
class NewCommandDeps:
    cancel_codex_reply_if_busy: CancelCodexReplyIfBusy
    resolve_new_thread_cwd: ResolveNewThreadCwd
    load_recent_threads: LoadRecentThreads
    spawn_background_new_thread_runner: SpawnBackgroundNewThreadRunner
    wait_for_new_thread: WaitForNewThread
    set_selected_thread_id: SetSelectedThreadId
    format_title_preview: FormatTitlePreview
    get_thread_ui_name: GetThreadUiName
    wait_for_prompt_delivery: WaitForPromptDelivery
    get_thread_label: GetThreadLabel
    sync_session_index_with_state: SyncSessionIndex
    print_line: PrintLine


def run_new_command(
    *,
    cwd: str | None,
    prompt: str | None,
    abort: bool,
    create_timeout: float,
    deps: NewCommandDeps,
) -> None:
    cancelled_labels, cancel_remaining = _cancel_if_requested(abort, deps.cancel_codex_reply_if_busy)
    target_cwd = deps.resolve_new_thread_cwd(cwd)
    prompt_text = prompt or ""
    if not prompt_text:
        raise NewCommandMissingPromptError()

    previous_ids = {thread.id for thread in deps.load_recent_threads(0)}
    runner = deps.spawn_background_new_thread_runner(prompt_text, target_cwd)
    thread = deps.wait_for_new_thread(previous_ids, create_timeout)
    if thread is None:
        _raise_missing_new_thread(runner)

    deps.print_line(f"[new_thread_detected_by_list_diff] {thread.id}")
    deps.set_selected_thread_id(thread.id)
    _print_new_thread_status(thread, target_cwd, cancelled_labels, cancel_remaining, deps)
    _verify_new_thread_delivery(thread, prompt_text, deps)
    _ = deps.sync_session_index_with_state()
    deps.print_line(f"[background_runner_pid] {runner.pid}")


def _cancel_if_requested(
    abort: bool,
    cancel_codex_reply_if_busy: CancelCodexReplyIfBusy,
) -> tuple[list[str], list[str]]:
    if not abort:
        return [], []
    return cancel_codex_reply_if_busy(3.0)


def _raise_missing_new_thread(runner: BackgroundRunner) -> NoReturn:
    exit_code = runner.poll()
    if exit_code is None:
        raise NewCommandThreadTimeoutError()
    raise NewCommandRunnerExitedError(exit_code)


def _print_new_thread_status(
    thread: ThreadInfo,
    target_cwd: str,
    cancelled_labels: list[str],
    cancel_remaining: list[str],
    deps: NewCommandDeps,
) -> None:
    deps.print_line(f"selected_thread: {thread.id}")
    deps.print_line(f"target_thread: {thread.id}")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"ui_name: {deps.get_thread_ui_name(thread.id, thread) or '-'}")
    deps.print_line(f"cwd: {thread.cwd or target_cwd}")
    deps.print_line("transport: local-sidecar runner (debug app-server send-message-v2)")
    if cancelled_labels:
        deps.print_line(f"reply_abort_requested: {', '.join(cancelled_labels)}")
        if cancel_remaining:
            deps.print_line(f"reply_abort_pending: {', '.join(cancel_remaining)}")


def _verify_new_thread_delivery(thread: ThreadInfo, prompt: str, deps: NewCommandDeps) -> None:
    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        raise NewCommandPromptDeliveryError()
    delivered_thread = deps.wait_for_prompt_delivery({thread.id: (thread, session_path, 0)}, prompt, 6.0)
    if delivered_thread is None:
        raise NewCommandPromptDeliveryError()
    deps.print_line(f"[delivery_verified] {deps.get_thread_label(thread)}")

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from codex_thread_models import ThreadInfo


LoadThreads = Callable[[int], list[ThreadInfo]]
GetThreadLabel = Callable[[ThreadInfo], str]
GetSelectedThreadId = Callable[[], str | None]
InterruptThread = Callable[[ThreadInfo], bool]
Sleep = Callable[[float], None]
TimeNow = Callable[[], float]


@dataclass(frozen=True, slots=True)
class ThreadActionDeps:
    load_recent_threads: LoadThreads
    get_busy_threads: LoadThreads
    get_thread_label: GetThreadLabel
    get_selected_thread_id: GetSelectedThreadId
    interrupt_thread_via_sidecar: InterruptThread
    time_now: TimeNow
    sleep: Sleep


def wait_for_new_thread(
    previous_ids: set[str],
    timeout_sec: float,
    deps: ThreadActionDeps,
) -> ThreadInfo | None:
    deadline = deps.time_now() + max(timeout_sec, 0.0)
    scan_limit = max(20, len(previous_ids) + 5)
    while deps.time_now() < deadline:
        for thread in deps.load_recent_threads(scan_limit):
            if thread.id not in previous_ids:
                return thread
        deps.sleep(0.25)
    return None


def cancel_codex_reply_if_busy(
    timeout_sec: float,
    deps: ThreadActionDeps,
) -> tuple[list[str], list[str]]:
    busy_before = deps.get_busy_threads(50)
    if not busy_before:
        return [], []

    labels_before = [deps.get_thread_label(thread) for thread in busy_before]
    selected_thread_id = deps.get_selected_thread_id()
    target_thread = _select_cancel_target(busy_before, selected_thread_id)
    if target_thread is None:
        return labels_before, labels_before

    try:
        _ = deps.interrupt_thread_via_sidecar(target_thread)
    except RuntimeError:
        return labels_before, labels_before

    deadline = deps.time_now() + timeout_sec
    remaining_threads = busy_before
    while deps.time_now() < deadline:
        remaining_threads = deps.get_busy_threads(50)
        if not any(thread.id == target_thread.id for thread in remaining_threads):
            return labels_before, [deps.get_thread_label(thread) for thread in remaining_threads]
        deps.sleep(0.2)

    return labels_before, [deps.get_thread_label(thread) for thread in remaining_threads]


def _select_cancel_target(
    busy_threads: list[ThreadInfo],
    selected_thread_id: str | None,
) -> ThreadInfo | None:
    if selected_thread_id:
        for thread in busy_threads:
            if thread.id == selected_thread_id:
                return thread
    if len(busy_threads) == 1:
        return busy_threads[0]
    return None

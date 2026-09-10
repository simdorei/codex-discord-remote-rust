from __future__ import annotations

import threading
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import codex_desktop_bridge_final_answer as final_answer_watch
from codex_thread_models import ThreadInfo


GetThreadLabel = Callable[[ThreadInfo], str]
PrintWatchStatus = Callable[[str, str], None]


class WatchForFinalAnswer(Protocol):
    def __call__(
        self,
        *,
        session_path: Path,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_live: bool = False,
        stream_label: str = "",
    ) -> final_answer_watch.WatchForFinalAnswerResult: ...


@dataclass(frozen=True, slots=True)
class BackgroundWatchDeps:
    get_thread_label: GetThreadLabel
    watch_for_final_answer: WatchForFinalAnswer
    print_watch_status: PrintWatchStatus


BACKGROUND_WATCHERS: dict[str, threading.Thread] = {}
BACKGROUND_WATCHERS_LOCK = threading.Lock()


def background_watch_worker(
    thread: ThreadInfo,
    start_offset: int,
    timeout_sec: float,
    include_commentary: bool,
    stream_output: bool,
    deps: BackgroundWatchDeps,
) -> None:
    label = deps.get_thread_label(thread)
    try:
        result = deps.watch_for_final_answer(
            session_path=Path(thread.rollout_path),
            start_offset=start_offset,
            timeout_sec=timeout_sec,
            include_commentary=include_commentary,
            stream_live=stream_output,
            stream_label=label,
        )
        if result["final_answer"]:
            deps.print_watch_status(label, "ready")
        elif result["status"] == "aborted":
            deps.print_watch_status(label, "aborted")
        elif result["status"] == "progress":
            deps.print_watch_status(label, "in_progress")
        elif result["status"] == "timeout":
            deps.print_watch_status(label, "watch_timeout")
        elif result["status"] == "failed":
            deps.print_watch_status(label, "failed")
        elif result["status"] == "transport_error":
            deps.print_watch_status(label, "transport_error")
    finally:
        with BACKGROUND_WATCHERS_LOCK:
            current = BACKGROUND_WATCHERS.get(thread.id)
            if current is threading.current_thread():
                _ = BACKGROUND_WATCHERS.pop(thread.id, None)


def start_background_watch(
    thread: ThreadInfo,
    start_offset: int,
    timeout_sec: float,
    include_commentary: bool,
    stream_output: bool,
    deps: BackgroundWatchDeps,
) -> bool:
    with BACKGROUND_WATCHERS_LOCK:
        existing = BACKGROUND_WATCHERS.get(thread.id)
        if existing and existing.is_alive():
            return False
        worker = threading.Thread(
            target=background_watch_worker,
            args=(thread, start_offset, timeout_sec, include_commentary, stream_output, deps),
            daemon=True,
            name=f"codex-bridge-watch-{thread.id[:8]}",
        )
        BACKGROUND_WATCHERS[thread.id] = worker
        worker.start()
        return True

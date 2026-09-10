# pyright: reportAny=false
from __future__ import annotations

import argparse
from pathlib import Path

from codex_desktop_bridge_command_ask_types import CommandAskDeps
from codex_desktop_bridge_final_answer_types import WatchForFinalAnswerResult as WatchResult
from codex_thread_models import ThreadInfo


def start_background_watch(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    thread: ThreadInfo,
    start_offset: int,
) -> int:
    started = deps.start_background_watch(
        thread=thread,
        start_offset=start_offset,
        timeout_sec=args.timeout,
        include_commentary=args.include_commentary,
        stream_output=args.stream,
    )
    if started:
        print(f"[background_watch_started] {deps.get_thread_label(thread)}")
    else:
        print(f"[background_watch_already_running] {deps.get_thread_label(thread)}")
    return 0


def wait_for_final_answer(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    session_path: Path,
    start_offset: int,
) -> int:
    print("[waiting_for_final_answer]")
    print("Use Ctrl+C to stop waiting after the prompt is sent.")

    try:
        result = deps.watch_for_final_answer(
            session_path=session_path,
            start_offset=start_offset,
            timeout_sec=args.timeout,
            include_commentary=args.include_commentary,
            stream_live=args.stream,
        )
    except KeyboardInterrupt:
        print("[wait_cancelled]")
        print("Prompt was already sent. Waiting stopped by user.")
        print("Use `status` or `tail --only-new` to monitor the same thread.")
        return 0

    commentary = result.get("commentary") or []
    _print_wait_commentary(
        include_commentary=args.include_commentary or result.get("status") == "progress",
        commentary=commentary,
        streamed_live=result.get("streamed_live") is True,
    )
    return _print_wait_result(result, commentary)


def _print_wait_commentary(
    *,
    include_commentary: bool,
    commentary: list[str],
    streamed_live: bool,
) -> None:
    if not include_commentary or not commentary or streamed_live:
        return
    for item in commentary:
        print("[commentary]")
        print(item)
        print("")


def _print_wait_result(result: WatchResult, commentary: list[str]) -> int:
    final_answer = result.get("final_answer") or ""
    if final_answer:
        if result.get("final_streamed_live"):
            print("[ready]")
        else:
            print("[final_answer]")
            print(final_answer)
            print("")
            print("[ready]")
        return 0

    if result.get("status") == "aborted":
        print("[aborted]")
        return 0

    if result.get("status") == "progress":
        print("[ready]")
        return 0

    if result.get("status") in {"failed", "transport_error"}:
        marker = "[failed]" if result.get("status") == "failed" else "[transport_error]"
        print(marker)
        print(result.get("error_message") or "Codex terminal state could not be verified.")
        return 1

    print("[timeout]")
    if commentary:
        print(commentary[-1])
    return 2

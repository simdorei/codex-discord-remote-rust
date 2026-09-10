from __future__ import annotations

from pathlib import Path

from codex_desktop_bridge_final_answer_events import process_watch_event
from codex_desktop_bridge_final_answer_native import resolve_native_watch
from codex_desktop_bridge_final_answer_types import (
    EmitWatchStreamBlock as EmitWatchStreamBlock,
    FinalAnswerWatchDeps as FinalAnswerWatchDeps,
    JsonObject as JsonObject,
    StreamCallback as StreamCallback,
    WatchForFinalAnswerResult as WatchForFinalAnswerResult,
    WatchState,
    result_from_state,
)


def emit_watch_stream_block(
    marker: str,
    text: str,
    *,
    stream_label: str = "",
    stream_callback: StreamCallback | None = None,
) -> None:
    prefix = f"{stream_label} " if stream_label else ""
    lines = [f"{prefix}{marker}", *str(text or "").splitlines(), ""]
    if stream_callback is not None:
        for line in lines:
            stream_callback(line)
        return
    for line in lines:
        print(line)


def watch_for_final_answer(
    session_path: Path,
    start_offset: int,
    timeout_sec: float,
    include_commentary: bool,
    *,
    deps: FinalAnswerWatchDeps,
    stream_live: bool = False,
    stream_label: str = "",
    stream_callback: StreamCallback | None = None,
    expected_turn_id: str | None = None,
) -> WatchForFinalAnswerResult:
    deadline = deps.time_now() + timeout_sec if timeout_sec > 0 else None
    cursor = start_offset
    state = WatchState()

    while deadline is None or deps.time_now() < deadline:
        if expected_turn_id is not None:
            native_result = resolve_native_watch(
                session_path=session_path,
                expected_turn_id=expected_turn_id,
                stream_live=stream_live,
                stream_label=stream_label,
                stream_callback=stream_callback,
                deps=deps,
                state=state,
            )
            if native_result is not None:
                return native_result
        events, cursor = deps.read_new_session_events(session_path, cursor)
        for event in events:
            result = process_watch_event(
                event,
                session_path=session_path,
                include_commentary=include_commentary,
                stream_live=stream_live,
                stream_label=stream_label,
                stream_callback=stream_callback,
                deps=deps,
                state=state,
                expected_turn_id=expected_turn_id,
            )
            if result is not None:
                return result

        if expected_turn_id is not None:
            native_result = resolve_native_watch(
                session_path=session_path,
                expected_turn_id=expected_turn_id,
                stream_live=stream_live,
                stream_label=stream_label,
                stream_callback=stream_callback,
                deps=deps,
                state=state,
            )
            if native_result is not None:
                return native_result

        deps.sleep(0.35)

    return result_from_state("timeout", state)

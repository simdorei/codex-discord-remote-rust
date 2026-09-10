from __future__ import annotations

from codex_desktop_bridge_final_answer_types import (
    FinalAnswerWatchDeps,
    JsonObject,
    StreamCallback,
    WatchState,
    append_commentary,
)


def process_interactive_response_item(
    payload_type: str,
    payload: JsonObject,
    *,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> bool:
    if payload_type == "function_call":
        _process_function_call_response_item(
            payload,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )
        return True

    if payload_type == "function_call_output":
        _process_function_call_output_response_item(
            payload,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )
        return True

    return False


def _process_function_call_response_item(
    payload: JsonObject,
    *,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> None:
    notice = deps.build_interactive_notice_from_function_call(payload)
    state.did_stream_live = append_commentary(
        notice,
        include_commentary=include_commentary,
        commentary=state.commentary,
        dedupe=state.seen_interactive_notices,
        stream_live=stream_live,
        stream_label=stream_label,
        stream_callback=stream_callback,
        deps=deps,
        did_stream_live=state.did_stream_live,
    )


def _process_function_call_output_response_item(
    payload: JsonObject,
    *,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> None:
    output_text = str(payload.get("output") or "").strip()
    if "rejected by user" not in output_text.lower():
        return

    state.did_stream_live = append_commentary(
        "[approval_rejected]\nCommand approval was rejected by user.",
        include_commentary=include_commentary,
        commentary=state.commentary,
        dedupe=state.seen_interactive_notices,
        stream_live=stream_live,
        stream_label=stream_label,
        stream_callback=stream_callback,
        deps=deps,
        did_stream_live=state.did_stream_live,
    )

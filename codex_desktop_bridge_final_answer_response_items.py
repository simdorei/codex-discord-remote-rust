from __future__ import annotations

from codex_desktop_bridge_final_answer_interactive_items import process_interactive_response_item
from codex_desktop_bridge_final_answer_types import (
    FinalAnswerWatchDeps,
    JsonObject,
    StreamCallback,
    WatchForFinalAnswerResult,
    WatchState,
    append_commentary,
)


def process_response_item(
    payload_type: str,
    payload: JsonObject,
    *,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> WatchForFinalAnswerResult | None:
    if process_interactive_response_item(
        payload_type,
        payload,
        include_commentary=include_commentary,
        stream_live=stream_live,
        stream_label=stream_label,
        stream_callback=stream_callback,
        deps=deps,
        state=state,
    ):
        return None

    if payload_type == "message":
        return _process_assistant_message_response_item(
            payload,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )

    return None


def _process_assistant_message_response_item(
    payload: JsonObject,
    *,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> WatchForFinalAnswerResult | None:
    if payload.get("role") != "assistant":
        return None

    text = deps.extract_message_text(payload)
    if not text:
        return None

    phase = payload.get("phase")
    if phase == "final_answer":
        state.final_answer = text
        return None

    if phase == "commentary" and include_commentary:
        state.did_stream_live = append_commentary(
            text,
            include_commentary=include_commentary,
            commentary=state.commentary,
            dedupe=None,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            did_stream_live=state.did_stream_live,
        )
    return None

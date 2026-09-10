from __future__ import annotations

from pathlib import Path

from codex_app_server_transport_goal import ThreadGoalStatus

from codex_desktop_bridge_final_answer_types import (
    FinalAnswerWatchDeps,
    JsonObject,
    StreamCallback,
    WatchForFinalAnswerResult,
    WatchState,
    append_commentary,
    result_from_state,
)
from codex_desktop_bridge_final_answer_response_items import process_response_item
from codex_session_events import JsonEvent


def process_watch_event(
    event: JsonEvent,
    *,
    session_path: Path,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
    expected_turn_id: str | None = None,
) -> WatchForFinalAnswerResult | None:
    payload = _event_payload(event)
    if payload is None:
        return None

    event_type = str(event.get("type") or "")
    payload_type = str(payload.get("type") or "")

    if event_type == "event_msg":
        return _process_event_msg(
            payload_type,
            payload,
            session_path=session_path,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
            expected_turn_id=expected_turn_id,
        )

    if event_type == "response_item":
        return process_response_item(
            payload_type,
            payload,
            include_commentary=include_commentary,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )

    return None


def _process_event_msg(
    payload_type: str,
    payload: JsonObject,
    *,
    session_path: Path,
    include_commentary: bool,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
    expected_turn_id: str | None,
) -> WatchForFinalAnswerResult | None:
    if payload_type == "agent_message":
        phase = str(payload.get("phase") or "")
        if phase == "final_answer":
            state.final_answer = str(payload.get("message") or "").strip()
            return None
        message = str(payload.get("message") or "").strip()
        state.did_stream_live = append_commentary(
            message,
            include_commentary=include_commentary,
            commentary=state.commentary,
            dedupe=state.seen_agent_messages,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            did_stream_live=state.did_stream_live,
        )
        return None

    if payload_type in {"turn_aborted", "task_aborted", "task_cancelled"}:
        if expected_turn_id is not None:
            return _record_rollout_terminal(
                payload_type,
                payload,
                expected_turn_id=expected_turn_id,
                deps=deps,
                state=state,
            )
        state.final_answer = ""
        return result_from_state("aborted", state)

    if payload_type == "task_complete":
        if expected_turn_id is not None:
            return _record_rollout_terminal(
                payload_type,
                payload,
                expected_turn_id=expected_turn_id,
                deps=deps,
                state=state,
            )
        return _complete_parent_turn(
            payload,
            session_path=session_path,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )

    return None


def _record_rollout_terminal(
    payload_type: str,
    payload: JsonObject,
    *,
    expected_turn_id: str,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> None:
    payload_turn_id = str(payload.get("turn_id") or "").strip()
    if payload_turn_id and payload_turn_id != expected_turn_id:
        return None
    state.rollout_terminal_type = payload_type
    state.rollout_terminal_payload = dict(payload)
    if state.rollout_observed_at is None:
        state.rollout_observed_at = deps.time_now()
    return None


def _complete_parent_turn(
    payload: JsonObject,
    *,
    session_path: Path,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> WatchForFinalAnswerResult:
    last_agent_message = payload.get("last_agent_message")
    if isinstance(last_agent_message, str) and last_agent_message.strip():
        state.final_answer = last_agent_message.strip()
    goal_status = deps.get_thread_goal_status(session_path)
    if goal_status not in {None, ThreadGoalStatus.COMPLETE}:
        progress_text = state.final_answer
        state.final_answer = ""
        state.did_stream_live = append_commentary(
            progress_text,
            include_commentary=True,
            commentary=state.commentary,
            dedupe=state.seen_agent_messages,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            did_stream_live=state.did_stream_live,
        )
        return result_from_state("progress", state)

    if stream_live and state.final_answer:
        state.did_stream_live = True
        state.did_stream_final_live = True
        deps.emit_watch_stream_block(
            "[final_answer]",
            state.final_answer,
            stream_label=stream_label,
            stream_callback=stream_callback,
        )
    return result_from_state("final", state)


def _event_payload(event: JsonEvent) -> JsonObject | None:
    payload = event.get("payload")
    if isinstance(payload, dict):
        return payload
    return None

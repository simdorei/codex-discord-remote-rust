from __future__ import annotations

from pathlib import Path

from codex_app_server_transport_goal import GoalAbsent, GoalTransportError, ThreadGoalStatus
from codex_app_server_transport_turn_outcomes import (
    TurnCompletionPending,
    TurnCompletionTransportError,
    TurnStatus,
)
from codex_desktop_bridge_final_answer_types import (
    FinalAnswerWatchDeps,
    JsonObject,
    StreamCallback,
    WatchForFinalAnswerResult,
    WatchState,
    append_commentary,
    result_from_state,
)

NATIVE_ROLLOUT_RECONCILE_SEC = 1.0
NATIVE_COMPLETION_TIMEOUT_SEC = 3.0
NO_VISIBLE_REPLY = "Codex turn completed without a visible reply."


def resolve_native_watch(
    *,
    session_path: Path,
    expected_turn_id: str,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> WatchForFinalAnswerResult | None:
    now = deps.time_now()
    observation = deps.observe_turn_completion(session_path, expected_turn_id)
    if isinstance(observation, TurnCompletionTransportError):
        state.final_answer = ""
        return result_from_state("transport_error", state, error_message=observation.message)
    if isinstance(observation, TurnCompletionPending):
        if state.rollout_observed_at is not None and now - state.rollout_observed_at >= NATIVE_COMPLETION_TIMEOUT_SEC:
            state.final_answer = ""
            return result_from_state(
                "transport_error",
                state,
                error_message="Timed out reconciling the Codex terminal state.",
            )
        return None
    completion = observation.completion
    state.native_completion = completion
    if completion.status is TurnStatus.INTERRUPTED:
        state.final_answer = ""
        origin = completion.interrupt_origin.value if completion.interrupt_origin is not None else ""
        return result_from_state("aborted", state, interrupt_origin=origin)
    if completion.status is TurnStatus.FAILED:
        state.final_answer = ""
        return result_from_state(
            "failed",
            state,
            error_message=completion.error_message or "Codex turn failed without an error message.",
        )

    if state.native_observed_at is None:
        state.native_observed_at = now
    if state.rollout_terminal_type == "task_complete" and state.rollout_terminal_payload is not None:
        return _complete_verified_native_turn(
            state.rollout_terminal_payload,
            session_path=session_path,
            expected_turn_id=expected_turn_id,
            stream_live=stream_live,
            stream_label=stream_label,
            stream_callback=stream_callback,
            deps=deps,
            state=state,
        )
    if now - state.native_observed_at < NATIVE_ROLLOUT_RECONCILE_SEC:
        return None
    return _complete_verified_native_turn(
        None,
        session_path=session_path,
        expected_turn_id=expected_turn_id,
        stream_live=stream_live,
        stream_label=stream_label,
        stream_callback=stream_callback,
        deps=deps,
        state=state,
    )


def _complete_verified_native_turn(
    payload: JsonObject | None,
    *,
    session_path: Path,
    expected_turn_id: str,
    stream_live: bool,
    stream_label: str,
    stream_callback: StreamCallback | None,
    deps: FinalAnswerWatchDeps,
    state: WatchState,
) -> WatchForFinalAnswerResult:
    exact_text = ""
    if payload is not None:
        last_agent_message = payload.get("last_agent_message")
        if isinstance(last_agent_message, str):
            exact_text = last_agent_message.strip()
    state.final_answer = exact_text

    goal_update = deps.get_thread_goal_update(session_path, expected_turn_id)
    if goal_update is not None:
        is_final = goal_update.status is ThreadGoalStatus.COMPLETE
    else:
        goal_lookup = deps.get_thread_goal_lookup(session_path)
        if isinstance(goal_lookup, GoalTransportError):
            state.final_answer = ""
            return result_from_state("transport_error", state, error_message=goal_lookup.message)
        is_final = isinstance(goal_lookup, GoalAbsent) or goal_lookup.status is ThreadGoalStatus.COMPLETE

    if not is_final:
        progress_text = exact_text or NO_VISIBLE_REPLY
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

    if not state.final_answer:
        state.final_answer = NO_VISIBLE_REPLY
    if stream_live:
        state.did_stream_live = True
        state.did_stream_final_live = True
        deps.emit_watch_stream_block(
            "[final_answer]",
            state.final_answer,
            stream_label=stream_label,
            stream_callback=stream_callback,
        )
    return result_from_state("final", state)

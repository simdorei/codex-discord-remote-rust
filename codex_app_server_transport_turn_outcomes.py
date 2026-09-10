from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum, unique
from collections.abc import Mapping
from typing import TypeAlias

from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject, JsonValue


@unique
class TurnStatus(StrEnum):
    COMPLETED = "completed"
    INTERRUPTED = "interrupted"
    FAILED = "failed"
    IN_PROGRESS = "inProgress"


@unique
class InterruptOrigin(StrEnum):
    REMOTE_USER_INTENT = "remote_user_intent"
    EXTERNAL_OR_UNKNOWN = "external_or_unknown"


@dataclass(frozen=True, slots=True)
class TurnCompletion:
    thread_id: str
    turn_id: str
    status: TurnStatus
    error_message: str = ""
    interrupt_origin: InterruptOrigin | None = None
    duration_ms: int | None = None


@dataclass(frozen=True, slots=True)
class TurnCompletionFound:
    completion: TurnCompletion


@dataclass(frozen=True, slots=True)
class TurnCompletionPending:
    pass


@dataclass(frozen=True, slots=True)
class TurnCompletionTransportError:
    message: str


TurnCompletionObservation: TypeAlias = (
    TurnCompletionFound | TurnCompletionPending | TurnCompletionTransportError
)


def parse_turn_completion_notification(
    params: Mapping[str, JsonValue],
    *,
    remote_user_intent: bool = False,
) -> TurnCompletion | None:
    thread_id = str(params.get("threadId") or "").strip()
    turn_value = params.get("turn")
    if not isinstance(turn_value, dict):
        return None
    return parse_turn_payload(
        thread_id,
        turn_value,
        remote_user_intent=remote_user_intent,
        require_terminal=True,
    )


def parse_thread_turn_completions(
    result: JsonObject,
    *,
    expected_thread_id: str,
) -> dict[str, TurnCompletion]:
    thread = result.get("thread")
    if not isinstance(thread, dict):
        raise CodexAppServerTransportError("thread/read returned an invalid thread payload.")
    thread_id = str(thread.get("id") or "").strip()
    if thread_id != expected_thread_id:
        raise CodexAppServerTransportError("thread/read returned a different thread.")
    turns = thread.get("turns")
    if not isinstance(turns, list):
        raise CodexAppServerTransportError("thread/read returned invalid turns.")
    completions: dict[str, TurnCompletion] = {}
    for turn in turns:
        if not isinstance(turn, dict):
            raise CodexAppServerTransportError("thread/read returned an invalid turn payload.")
        completion = parse_turn_payload(thread_id, turn, require_terminal=False)
        if completion is not None:
            completions[completion.turn_id] = completion
    return completions


def parse_thread_turn_states(
    result: JsonObject,
    *,
    expected_thread_id: str,
) -> dict[str, TurnCompletion]:
    thread_id, turns = _parse_thread_turn_list(result, expected_thread_id=expected_thread_id)
    states: dict[str, TurnCompletion] = {}
    for turn in turns:
        state = parse_turn_payload(
            thread_id,
            turn,
            require_terminal=False,
            include_in_progress=True,
        )
        if state is not None:
            states[state.turn_id] = state
    return states


def parse_turn_payload(
    thread_id: str,
    turn: Mapping[str, JsonValue],
    *,
    remote_user_intent: bool = False,
    require_terminal: bool,
    include_in_progress: bool = False,
) -> TurnCompletion | None:
    if not thread_id:
        raise CodexAppServerTransportError("turn payload had no thread id.")
    turn_id = str(turn.get("id") or "").strip()
    if not turn_id:
        raise CodexAppServerTransportError("turn payload had no turn id.")
    status_value = turn.get("status")
    if not isinstance(status_value, str):
        raise CodexAppServerTransportError("turn payload had no status.")
    try:
        status = TurnStatus(status_value)
    except ValueError as exc:
        raise CodexAppServerTransportError(f"turn payload had an unknown status: {status_value}") from exc
    if status is TurnStatus.IN_PROGRESS:
        if require_terminal:
            raise CodexAppServerTransportError("turn/completed carried an inProgress turn.")
        if not include_in_progress:
            return None
    error_message = _parse_error_message(turn, status)
    duration_value = turn.get("durationMs")
    duration_ms = duration_value if isinstance(duration_value, int) and not isinstance(duration_value, bool) else None
    interrupt_origin = None
    if status is TurnStatus.INTERRUPTED:
        interrupt_origin = (
            InterruptOrigin.REMOTE_USER_INTENT
            if remote_user_intent
            else InterruptOrigin.EXTERNAL_OR_UNKNOWN
        )
    return TurnCompletion(
        thread_id=thread_id,
        turn_id=turn_id,
        status=status,
        error_message=error_message,
        interrupt_origin=interrupt_origin,
        duration_ms=duration_ms,
    )


def _parse_thread_turn_list(
    result: JsonObject,
    *,
    expected_thread_id: str,
) -> tuple[str, list[JsonObject]]:
    thread = result.get("thread")
    if not isinstance(thread, dict):
        raise CodexAppServerTransportError("thread/read returned an invalid thread payload.")
    thread_id = str(thread.get("id") or "").strip()
    if thread_id != expected_thread_id:
        raise CodexAppServerTransportError("thread/read returned a different thread.")
    turns = thread.get("turns")
    if not isinstance(turns, list):
        raise CodexAppServerTransportError("thread/read returned invalid turns.")
    parsed_turns: list[JsonObject] = []
    for turn in turns:
        if not isinstance(turn, dict):
            raise CodexAppServerTransportError("thread/read returned an invalid turn payload.")
        parsed_turns.append(turn)
    return thread_id, parsed_turns


def _parse_error_message(turn: Mapping[str, JsonValue], status: TurnStatus) -> str:
    error = turn.get("error")
    if error is None:
        return ""
    if not isinstance(error, dict):
        raise CodexAppServerTransportError("turn payload had an invalid error.")
    message = error.get("message")
    if not isinstance(message, str):
        raise CodexAppServerTransportError("turn payload error had no message.")
    clean_message = message.strip()
    if status is not TurnStatus.FAILED:
        return ""
    return clean_message[:1000]

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum, unique
from collections.abc import Mapping
from typing import TypeAlias

from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject, JsonValue


@unique
class ThreadGoalStatus(StrEnum):
    ACTIVE = "active"
    PAUSED = "paused"
    BLOCKED = "blocked"
    USAGE_LIMITED = "usageLimited"
    BUDGET_LIMITED = "budgetLimited"
    COMPLETE = "complete"


def is_terminal_goal_status(status: ThreadGoalStatus) -> bool:
    return status in {
        ThreadGoalStatus.BLOCKED,
        ThreadGoalStatus.COMPLETE,
    }


@dataclass(frozen=True, slots=True)
class GoalAbsent:
    pass


@dataclass(frozen=True, slots=True)
class GoalPresent:
    status: ThreadGoalStatus


@dataclass(frozen=True, slots=True)
class GoalTransportError:
    message: str


@dataclass(frozen=True, slots=True)
class ThreadGoalUpdate:
    thread_id: str
    turn_id: str | None
    status: ThreadGoalStatus


ThreadGoalLookup: TypeAlias = GoalAbsent | GoalPresent | GoalTransportError


def parse_thread_goal_status(
    result: JsonObject,
    *,
    expected_thread_id: str,
) -> ThreadGoalStatus | None:
    goal = result.get("goal")
    if goal is None:
        return None
    if not isinstance(goal, dict):
        raise CodexAppServerTransportError("thread/goal/get returned an invalid goal payload.")
    if goal.get("threadId") != expected_thread_id:
        raise CodexAppServerTransportError("thread/goal/get returned a goal for a different thread.")
    status = goal.get("status")
    if not isinstance(status, str):
        raise CodexAppServerTransportError("thread/goal/get returned an invalid goal status.")
    try:
        return ThreadGoalStatus(status)
    except ValueError as exc:
        raise CodexAppServerTransportError(
            f"thread/goal/get returned an unknown goal status: {status}"
        ) from exc


def parse_thread_goal_update(params: Mapping[str, JsonValue]) -> ThreadGoalUpdate:
    thread_id = str(params.get("threadId") or "").strip()
    if not thread_id:
        raise CodexAppServerTransportError("thread/goal/updated had no thread id.")
    goal = params.get("goal")
    if not isinstance(goal, dict):
        raise CodexAppServerTransportError("thread/goal/updated had an invalid goal payload.")
    goal_thread_id = str(goal.get("threadId") or "").strip()
    if goal_thread_id != thread_id:
        raise CodexAppServerTransportError("thread/goal/updated carried a goal for a different thread.")
    status_value = goal.get("status")
    if not isinstance(status_value, str):
        raise CodexAppServerTransportError("thread/goal/updated had an invalid goal status.")
    try:
        status = ThreadGoalStatus(status_value)
    except ValueError as exc:
        raise CodexAppServerTransportError(
            f"thread/goal/updated had an unknown goal status: {status_value}"
        ) from exc
    turn_id_value = params.get("turnId")
    turn_id = turn_id_value.strip() if isinstance(turn_id_value, str) and turn_id_value.strip() else None
    return ThreadGoalUpdate(thread_id=thread_id, turn_id=turn_id, status=status)

from __future__ import annotations

import time
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
ReplyResult: TypeAlias = JsonObject
AnswersByQuestion: TypeAlias = dict[str, list[str]]

GetPendingRequest = Callable[["ReplyThread", float], JsonObject | None]
GetBusyState = Callable[["ReplyThread", bool], str]
BuildInputPayload = Callable[[JsonObject, str], tuple[JsonObject, AnswersByQuestion]]
SubmitUserInput = Callable[["ReplyThread", str, JsonObject, float], ReplyResult]
GetCachedApproval = Callable[[str], JsonObject | None]
GetPermissionApproval = Callable[[Path], JsonObject | None]
SubmitPermissionApproval = Callable[[str], ReplyResult]
BuildApprovalPayload = Callable[[str], tuple[str, str]]
BuildApprovalCandidates = Callable[[str], Sequence[JsonValue]]
SubmitApprovalDecision = Callable[["ReplyThread", JsonValue, JsonValue, str, float, bool], ReplyResult]
ClearCachedApproval = Callable[[str], None]
TimeNow = Callable[[], float]
Sleep = Callable[[float], None]


class ReplyThread(Protocol):
    @property
    def id(self) -> str: ...

    @property
    def rollout_path(self) -> str: ...


@dataclass(frozen=True, slots=True)
class PendingReplyDeps:
    get_pending_user_input_request: GetPendingRequest
    get_pending_approval_request: GetPendingRequest
    get_thread_busy_state: GetBusyState
    build_reply_input_response_payload: BuildInputPayload
    submit_user_input: SubmitUserInput
    get_cached_live_approval_request: GetCachedApproval
    get_pending_permission_approval_from_session: GetPermissionApproval
    submit_permission_approval_via_ui_row_select: SubmitPermissionApproval
    build_approval_decision_payload: BuildApprovalPayload
    build_approval_decision_candidate_payloads: BuildApprovalCandidates
    submit_approval_decision: SubmitApprovalDecision
    clear_cached_live_approval_request: ClearCachedApproval
    time_now: TimeNow = time.time
    sleep: Sleep = time.sleep

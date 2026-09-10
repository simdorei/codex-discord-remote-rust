from __future__ import annotations

from codex_desktop_bridge_reply_approval import (
    PendingApprovalMissingError,
    PendingApprovalRequestIdMissingError,
    PendingApprovalRequestKindMissingError,
    PendingApprovalSnapshotMissingError,
    reply_to_pending_approval,
)
from codex_desktop_bridge_reply_types import (
    AnswersByQuestion,
    BuildApprovalCandidates,
    BuildApprovalPayload,
    BuildInputPayload,
    ClearCachedApproval,
    GetBusyState,
    GetCachedApproval,
    GetPendingRequest,
    GetPermissionApproval,
    JsonObject,
    JsonScalar,
    JsonValue,
    PendingReplyDeps,
    ReplyResult,
    ReplyThread,
    Sleep,
    SubmitApprovalDecision,
    SubmitPermissionApproval,
    SubmitUserInput,
    TimeNow,
)

__all__ = [
    "AnswersByQuestion",
    "BuildApprovalCandidates",
    "BuildApprovalPayload",
    "BuildInputPayload",
    "ClearCachedApproval",
    "GetBusyState",
    "GetCachedApproval",
    "GetPendingRequest",
    "GetPermissionApproval",
    "JsonObject",
    "JsonScalar",
    "JsonValue",
    "PendingApprovalMissingError",
    "PendingApprovalRequestIdMissingError",
    "PendingApprovalRequestKindMissingError",
    "PendingApprovalSnapshotMissingError",
    "PendingReplyDeps",
    "PendingUserInputMissingError",
    "PendingUserInputRequestIdMissingError",
    "PendingUserInputSnapshotMissingError",
    "ReplyResult",
    "ReplyThread",
    "Sleep",
    "SubmitApprovalDecision",
    "SubmitPermissionApproval",
    "SubmitUserInput",
    "TimeNow",
    "reply_to_pending_approval",
    "reply_to_pending_user_input",
]


class PendingUserInputSnapshotMissingError(RuntimeError):
    pass


class PendingUserInputMissingError(RuntimeError):
    pass


class PendingUserInputRequestIdMissingError(RuntimeError):
    pass


def reply_to_pending_user_input(
    thread: ReplyThread,
    answer_text: str,
    timeout_sec: float,
    deps: PendingReplyDeps,
) -> ReplyResult:
    pending_request = deps.get_pending_user_input_request(thread, timeout_sec)
    if pending_request is None:
        busy_state = deps.get_thread_busy_state(thread, True)
        if busy_state == "waiting-input":
            raise PendingUserInputSnapshotMissingError(
                "The thread still looks like waiting-input, but no pending input request snapshot "
                + "was received over IPC. Open the thread once in Codex Desktop and retry."
            )
        raise PendingUserInputMissingError("No pending user input request is active for the selected thread.")

    request_id = _clean_string(pending_request.get("request_id"))
    if not request_id:
        raise PendingUserInputRequestIdMissingError("The pending input request did not include a request id.")

    response_payload, answers_by_question = deps.build_reply_input_response_payload(
        pending_request,
        answer_text,
    )
    result = deps.submit_user_input(thread, request_id, response_payload, timeout_sec)
    _ = result.setdefault("request_id", request_id)
    result["answers_by_question"] = _answers_by_question_json(answers_by_question)
    return result


def _clean_string(value: JsonValue | None) -> str:
    return value.strip() if isinstance(value, str) else ""


def _answers_by_question_json(answers_by_question: AnswersByQuestion) -> JsonObject:
    result: JsonObject = {}
    for question_id, answers in answers_by_question.items():
        answer_values: list[JsonValue] = []
        for answer in answers:
            answer_values.append(answer)
        result[question_id] = answer_values
    return result
